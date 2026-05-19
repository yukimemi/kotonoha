//! Static SPA served from `rust-embed`. The `web/dist/` folder is
//! baked into the binary at compile time so a single `kotonoha`
//! binary ships both the API and the React UI — `cargo install
//! kotonoha-server` is now a self-contained install.
//!
//! Fallback semantics (intentional, matches kanade-backend):
//! **any** unmatched path served by this handler returns
//! `index.html` with a 200. That includes "looks-like-static-asset"
//! paths such as `/missing.png` — the SPA's client-side router
//! decides what to do (typically: "page not found" component).
//! API + WebSocket routes are registered before this fallback in
//! `main.rs`, so a missing API endpoint still returns its real
//! 4xx without reaching this handler.
//!
//! Layout cribbed from kanade-backend's web.rs.

use axum::body::Body;
use axum::extract::Request;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "web/dist/"]
struct WebAssets;

pub async fn serve(req: Request) -> Response {
    let path = req.uri().path().trim_start_matches('/');
    let lookup = if path.is_empty() { "index.html" } else { path };

    if let Some(content) = WebAssets::get(lookup) {
        let mime = mime_guess::from_path(lookup).first_or_octet_stream();
        // `content.data` is a `Cow<'static, [u8]>`; pass it straight
        // into the body to skip the `into_owned()` clone. (Gemini
        // PR #38 perf nit.)
        return ([(header::CONTENT_TYPE, mime.as_ref())], content.data).into_response();
    }

    // SPA fallback: any unmatched non-API path → index.html so the
    // client-side router takes over.
    if let Some(idx) = WebAssets::get("index.html") {
        let mime = mime_guess::from_ext("html").first_or_octet_stream();
        return ([(header::CONTENT_TYPE, mime.as_ref())], idx.data).into_response();
    }

    (StatusCode::NOT_FOUND, Body::empty()).into_response()
}
