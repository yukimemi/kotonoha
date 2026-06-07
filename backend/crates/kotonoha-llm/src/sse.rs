//! SSE plumbing shared by the streaming HTTP backends.
//!
//! Every provider speaks the same transport shape — an SSE byte stream
//! whose `data:` payloads are JSON events carrying optional text — and
//! differs only in the event schema. [`parse_text_stream`] owns the
//! transport loop (buffering, event-boundary splitting, `[DONE]` /
//! keepalive handling, latency logging); each backend supplies a
//! payload→text closure for its schema.

use anyhow::{Context as _, anyhow};
use async_stream::try_stream;
use bytes::Bytes;
use futures::{Stream, StreamExt};

/// Find the next SSE event boundary in `buf` and return `(position,
/// separator_length)`.  Accepts both `\n\n` (spec) and `\r\n\r\n`
/// (some intermediaries); when both appear, the earliest one wins so
/// mixed-style buffers still split event-by-event.
pub(crate) fn find_event_boundary(buf: &[u8]) -> Option<(usize, usize)> {
    let crlf = buf.windows(4).position(|w| w == b"\r\n\r\n");
    let lf = buf.windows(2).position(|w| w == b"\n\n");
    match (crlf, lf) {
        (Some(c), Some(l)) if c < l => Some((c, 4)),
        (Some(_), Some(l)) => Some((l, 2)),
        (Some(c), None) => Some((c, 4)),
        (None, Some(l)) => Some((l, 2)),
        (None, None) => None,
    }
}

/// Truncate `s` to at most `max` bytes on a char boundary, appending an
/// ellipsis when cut.  Used to keep provider error bodies bounded —
/// these errors travel to the browser over the WebSocket, and a
/// hostile/misconfigured endpoint could echo arbitrary content (even
/// reflected request headers) back in its body.
pub(crate) fn snippet(s: &str, max: usize) -> String {
    let s = s.trim();
    if s.len() <= max {
        return s.to_owned();
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

/// Drain every *complete* SSE event currently in `buffer`, feed each
/// `data:` payload through `parse_payload`, and push extracted text
/// chunks onto `texts`.  `parsed` counts payloads that parsed
/// successfully (with or without text) so the caller can distinguish
/// "valid stream with an empty reply" from "not SSE at all".
///
/// Lines without a `data:` prefix — keepalive comments like
/// OpenRouter's `: PROCESSING`, `event:` fields — are skipped, as are
/// empty payloads and the OpenAI-style `[DONE]` terminator.
///
/// Note: each `data:` line is parsed as a complete JSON document.  The
/// SSE spec allows one logical payload to span multiple `data:` lines,
/// but no provider in scope (Gemini / OpenAI dialect) does that.
fn drain_complete_events<F>(
    buffer: &mut Vec<u8>,
    provider: &str,
    parse_payload: &F,
    parsed: &mut usize,
    texts: &mut Vec<String>,
) where
    F: Fn(&str) -> Result<Option<String>, serde_json::Error>,
{
    while let Some((pos, sep_len)) = find_event_boundary(buffer) {
        let event_bytes = buffer.drain(..pos + sep_len).collect::<Vec<u8>>();
        let event = String::from_utf8_lossy(&event_bytes);
        for line in event.lines() {
            let Some(payload) = line.strip_prefix("data:") else {
                continue;
            };
            let payload = payload.trim();
            if payload.is_empty() || payload == "[DONE]" {
                continue;
            }
            match parse_payload(payload) {
                Ok(maybe_text) => {
                    *parsed += 1;
                    if let Some(text) = maybe_text {
                        texts.push(text);
                    }
                }
                Err(e) => {
                    tracing::warn!(target: "kotonoha::llm",
                        "{provider} parse skipped: {e} on `{payload}`");
                }
            }
        }
    }
}

/// Drive an SSE byte stream into a stream of reply-text chunks.
///
/// `t_send` is the instant the HTTP request was sent — used for the
/// ttfb/ttft log line.  `parse_payload` maps one `data:` payload (a
/// JSON event) to its optional text chunk; a `serde_json::Error` is
/// logged and the payload skipped.
///
/// The stream errors only when *nothing* in the response parsed as an
/// event (wrong model name, non-SSE body, HTML error page).  A valid
/// stream that happens to carry no text (empty reply, safety block) is
/// surfaced as a normal empty stream — that's a model outcome, not a
/// transport failure.
pub(crate) fn parse_text_stream<S, F>(
    provider: String,
    t_send: std::time::Instant,
    mut byte_stream: S,
    parse_payload: F,
) -> impl Stream<Item = anyhow::Result<String>> + Send
where
    S: Stream<Item = reqwest::Result<Bytes>> + Unpin + Send + 'static,
    F: Fn(&str) -> Result<Option<String>, serde_json::Error> + Send + 'static,
{
    try_stream! {
        let mut buffer: Vec<u8> = Vec::new();
        let mut texts: Vec<String> = Vec::new();
        let mut total_bytes: usize = 0;
        let mut parsed: usize = 0;
        let mut yielded: usize = 0;
        let mut first_chunk_at: Option<std::time::Duration> = None;
        let mut first_yield_at: Option<std::time::Duration> = None;
        while let Some(chunk) = byte_stream.next().await {
            let chunk: Bytes = chunk.context("stream chunk")?;
            if first_chunk_at.is_none() {
                first_chunk_at = Some(t_send.elapsed());
            }
            total_bytes += chunk.len();
            buffer.extend_from_slice(&chunk);
            drain_complete_events(&mut buffer, &provider, &parse_payload, &mut parsed, &mut texts);
            for text in texts.drain(..) {
                if first_yield_at.is_none() {
                    first_yield_at = Some(t_send.elapsed());
                }
                yielded += text.len();
                yield text;
            }
        }
        // Tail flush — the final event may not end with a blank line.
        // Synthesize the terminator and run the same drain once more.
        if !buffer.is_empty() {
            buffer.extend_from_slice(b"\n\n");
            drain_complete_events(&mut buffer, &provider, &parse_payload, &mut parsed, &mut texts);
            for text in texts.drain(..) {
                yielded += text.len();
                yield text;
            }
        }
        let total = t_send.elapsed();
        tracing::info!(
            target: "kotonoha::llm",
            "{provider} stream done: total={:.0}ms ttfb={:.0}ms ttft={:.0}ms bytes={total_bytes} events={parsed} chars={yielded}",
            total.as_secs_f64() * 1000.0,
            first_chunk_at.map(|d| d.as_secs_f64() * 1000.0).unwrap_or(0.0),
            first_yield_at.map(|d| d.as_secs_f64() * 1000.0).unwrap_or(0.0),
        );
        if parsed == 0 {
            // Nothing in the response parsed as an event.  Most likely
            // the model name is wrong or the endpoint returned a
            // non-SSE body.  Surface this rather than letting the chat
            // sit silent.
            Err(anyhow!(
                "{provider} stream contained no parseable events ({total_bytes} bytes \
                 received). Check model name and that the endpoint returned SSE."
            ))?;
        } else if yielded == 0 {
            // Events parsed but carried no text — an empty reply or a
            // provider-side block.  Legitimate model outcome; log it
            // server-side but don't error the turn.
            tracing::warn!(target: "kotonoha::llm",
                "{provider} stream parsed {parsed} events but produced no text");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_lf_lf_boundary() {
        assert_eq!(find_event_boundary(b"data: a\n\ndata: b"), Some((7, 2)));
    }

    #[test]
    fn finds_crlf_crlf_boundary() {
        assert_eq!(find_event_boundary(b"data: a\r\n\r\nrest"), Some((7, 4)));
    }

    #[test]
    fn earliest_boundary_wins_when_styles_mix() {
        // LF-LF event first, CRLF-CRLF event later — must split at the
        // first boundary, not jump ahead to the CRLF one.
        assert_eq!(
            find_event_boundary(b"data: a\n\ndata: b\r\n\r\n"),
            Some((7, 2))
        );
        // And the reverse order.
        assert_eq!(
            find_event_boundary(b"data: a\r\n\r\ndata: b\n\n"),
            Some((7, 4))
        );
    }

    #[test]
    fn boundary_at_start() {
        assert_eq!(find_event_boundary(b"\n\nx"), Some((0, 2)));
    }

    #[test]
    fn none_when_incomplete() {
        assert_eq!(find_event_boundary(b"data: partial"), None);
    }

    #[test]
    fn snippet_passes_short_strings_through() {
        assert_eq!(snippet("  hello  ", 500), "hello");
    }

    #[test]
    fn snippet_truncates_on_char_boundary() {
        // "あ" is 3 bytes; cutting at 4 must back off to the boundary.
        let s = snippet("ああ", 4);
        assert_eq!(s, "あ…");
    }

    /// Closure used by the stream tests: `{"t": "..."}` → text,
    /// `{"t": null}` → parsed-but-empty.
    fn parse_t(payload: &str) -> Result<Option<String>, serde_json::Error> {
        serde_json::from_str::<serde_json::Value>(payload)
            .map(|v| v.get("t").and_then(|t| t.as_str()).map(str::to_owned))
    }

    fn collect(chunks: Vec<&'static [u8]>) -> Vec<anyhow::Result<String>> {
        let byte_stream = futures::stream::iter(
            chunks
                .into_iter()
                .map(|c| Ok::<_, reqwest::Error>(Bytes::from_static(c)))
                .collect::<Vec<_>>(),
        );
        let stream = parse_text_stream(
            "test".into(),
            std::time::Instant::now(),
            byte_stream,
            parse_t,
        );
        futures::executor::block_on(async {
            let stream = std::pin::pin!(stream);
            stream.collect::<Vec<_>>().await
        })
    }

    #[test]
    fn end_to_end_with_keepalives_split_chunks_and_done() {
        // Keepalive comment, role-only event, an event split across
        // two chunks, a CRLF event, [DONE] — only the text survives.
        let out = collect(vec![
            b": PROCESSING\n\ndata: {\"t\":null}\n\nda",
            b"ta: {\"t\":\"Hello\"}\n\ndata: {\"t\":\" world\"}\r\n\r\ndata: [DONE]\n\n",
        ]);
        let texts: Vec<_> = out.into_iter().map(|r| r.unwrap()).collect();
        assert_eq!(texts, vec!["Hello", " world"]);
    }

    #[test]
    fn tail_flush_recovers_final_unterminated_event() {
        let out = collect(vec![b"data: {\"t\":\"tail\"}"]);
        let texts: Vec<_> = out.into_iter().map(|r| r.unwrap()).collect();
        assert_eq!(texts, vec!["tail"]);
    }

    #[test]
    fn errors_when_nothing_parses() {
        let out = collect(vec![b"<html>not sse at all</html>"]);
        assert_eq!(out.len(), 1);
        let err = out.into_iter().next().unwrap().unwrap_err();
        assert!(err.to_string().contains("no parseable events"), "{err}");
    }

    #[test]
    fn empty_reply_is_not_an_error() {
        // Valid SSE whose events carry no text — must end cleanly.
        let out = collect(vec![b"data: {\"t\":null}\n\ndata: [DONE]\n\n"]);
        assert!(out.is_empty());
    }
}
