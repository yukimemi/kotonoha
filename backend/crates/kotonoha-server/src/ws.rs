use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::IntoResponse;
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};

use kotonoha_core::{Backend, BackendConfig, CliBackend, CompletionRequest, Lesson, Session};

use crate::AppState;

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientMsg {
    Configure {
        backend: Option<String>,
        lesson: Option<String>,
    },
    User {
        text: String,
    },
    Reset,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ServerMsg<'a> {
    Ready { backend: &'a str, lesson: &'a str },
    Delta { text: String },
    Done,
    Error { message: String },
}

pub async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: AppState) {
    let cfg = state.config.clone();
    let mut backend_name = cfg.default_backend_name().to_string();
    let mut lesson_name = cfg.default_lesson_name().to_string();
    let mut session = Session::default();

    let mut lesson = match cfg.load_lesson(&lesson_name) {
        Ok(l) => l,
        Err(e) => {
            tracing::error!("load default lesson: {e}");
            return;
        }
    };

    let (mut tx, mut rx) = socket.split();
    let _ = tx
        .send(Message::Text(
            serde_json::to_string(&ServerMsg::Ready {
                backend: &backend_name,
                lesson: &lesson_name,
            })
            .unwrap()
            .into(),
        ))
        .await;

    while let Some(Ok(msg)) = rx.next().await {
        let text = match msg {
            Message::Text(t) => t,
            Message::Close(_) => break,
            _ => continue,
        };

        let parsed: Result<ClientMsg, _> = serde_json::from_str(&text);
        let Ok(msg) = parsed else {
            let _ = send_err(&mut tx, format!("bad message: {text}")).await;
            continue;
        };

        match msg {
            ClientMsg::Configure {
                backend,
                lesson: lesson_opt,
            } => {
                if let Some(b) = backend {
                    if cfg.backend.contains_key(&b) {
                        backend_name = b;
                    } else {
                        let _ = send_err(&mut tx, format!("unknown backend: {b}")).await;
                        continue;
                    }
                }
                if let Some(l) = lesson_opt {
                    match cfg.load_lesson(&l) {
                        Ok(new) => {
                            lesson = new;
                            lesson_name = l;
                            session.reset();
                        }
                        Err(e) => {
                            let _ = send_err(&mut tx, format!("load lesson: {e}")).await;
                            continue;
                        }
                    }
                }
                let _ = tx
                    .send(Message::Text(
                        serde_json::to_string(&ServerMsg::Ready {
                            backend: &backend_name,
                            lesson: &lesson_name,
                        })
                        .unwrap()
                        .into(),
                    ))
                    .await;
            }
            ClientMsg::Reset => {
                session.reset();
            }
            ClientMsg::User { text } => {
                session.push_student(&text);
                if let Err(e) = run_turn(&cfg, &backend_name, &lesson, &mut session, &mut tx).await
                {
                    let _ = send_err(&mut tx, format!("{e:#}")).await;
                }
            }
        }
    }
}

async fn run_turn(
    cfg: &kotonoha_core::Config,
    backend_name: &str,
    lesson: &Lesson,
    session: &mut Session,
    tx: &mut futures::stream::SplitSink<WebSocket, Message>,
) -> anyhow::Result<()> {
    let backend_cfg = cfg
        .backend
        .get(backend_name)
        .ok_or_else(|| anyhow::anyhow!("backend not configured: {backend_name}"))?;
    let backend: Box<dyn Backend> = match backend_cfg {
        BackendConfig::Cli(c) => Box::new(CliBackend::from(c)),
        BackendConfig::Api(a) => kotonoha_llm::build_backend(a)?,
    };

    let request = CompletionRequest {
        system_prompt: lesson.system_prompt.clone(),
        turns: session.turns.clone(),
    };
    let mut stream = backend.complete(request).await?;
    use futures::StreamExt as _;

    let mut full = String::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        full.push_str(&chunk);
        let _ = tx
            .send(Message::Text(
                serde_json::to_string(&ServerMsg::Delta { text: chunk })
                    .unwrap()
                    .into(),
            ))
            .await;
    }
    session.push_teacher(full.trim());
    let _ = tx
        .send(Message::Text(
            serde_json::to_string(&ServerMsg::Done).unwrap().into(),
        ))
        .await;
    Ok(())
}

async fn send_err(
    tx: &mut futures::stream::SplitSink<WebSocket, Message>,
    message: String,
) -> Result<(), axum::Error> {
    tx.send(Message::Text(
        serde_json::to_string(&ServerMsg::Error { message })
            .unwrap()
            .into(),
    ))
    .await
}
