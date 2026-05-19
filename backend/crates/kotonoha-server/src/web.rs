//! Static SPA served from `rust-embed`. The `web/dist/` folder is
//! baked into the binary at compile time so a single `kotonoha`
//! binary ships both the API and the React UI — `cargo install
//! kotonoha-server` is now a self-contained install.
//!
//! Fallback semantics:
//!
//! - **Non-API path that matches an embedded asset** → return
//!   that asset with the correct content-type.
//! - **Non-API path with no embedded match** → return
//!   `index.html` so the SPA's client-side router takes over
//!   ("page not found" component etc.).
//! - **`/api/...` or `/ws/...` with no real backend route** →
//!   plain 404. axum's `.fallback()` catches *every* unmatched
//!   request, so without this guard a typo like `/api/inf` would
//!   silently get HTML back from this handler instead of a 404
//!   the client could detect.
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

    // API / WS namespaces never fall back to the SPA — a missing
    // route there is a real 404, not "show the dashboard".
    if path.starts_with("api/") || path.starts_with("ws/") {
        return (StatusCode::NOT_FOUND, Body::empty()).into_response();
    }

    let lookup = if path.is_empty() { "index.html" } else { path };

    if let Some(content) = WebAssets::get(lookup) {
        let mime = mime_guess::from_path(lookup).first_or_octet_stream();
        // `content.data` is a `Cow<'static, [u8]>`; passing it
        // directly to the body avoids an unnecessary clone.
        return ([(header::CONTENT_TYPE, mime.as_ref())], content.data).into_response();
    }

    // SPA fallback: any other unmatched path → index.html so the
    // client-side router takes over.
    if let Some(idx) = WebAssets::get("index.html") {
        let mime = mime_guess::from_ext("html").first_or_octet_stream();
        return ([(header::CONTENT_TYPE, mime.as_ref())], idx.data).into_response();
    }

    (StatusCode::NOT_FOUND, Body::empty()).into_response()
}
