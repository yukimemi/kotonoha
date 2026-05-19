//! Static SPA served from `rust-embed`. The `web/dist/` folder is
//! baked into the binary at compile time so a single `kotonoha`
//! binary ships both the API and the React UI — `cargo install
//! kotonoha-server` is now a self-contained install.
//!
//! Unmatched non-API routes fall back to `index.html` so client-side
//! routing in the SPA survives a full reload. Real 404s for static
//! assets (e.g. `/missing.png`) are still served with a 404 — only
//! HTML / navigation requests trigger the fallback.
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
        return (
            [(header::CONTENT_TYPE, mime.as_ref())],
            content.data.into_owned(),
        )
            .into_response();
    }

    // SPA fallback: any unmatched non-API path → index.html so the
    // client-side router takes over.
    if let Some(idx) = WebAssets::get("index.html") {
        let mime = mime_guess::from_ext("html").first_or_octet_stream();
        return (
            [(header::CONTENT_TYPE, mime.as_ref())],
            idx.data.into_owned(),
        )
            .into_response();
    }

    (StatusCode::NOT_FOUND, Body::empty()).into_response()
}
