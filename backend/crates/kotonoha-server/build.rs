// `rust-embed` reads `web/dist/` at compile time. The kata-managed
// `.github/workflows/ci.yml` doesn't (and shouldn't) know about our
// SPA build step — that lives only in release.yml + Makefile.toml's
// `web-build` task — so a plain `cargo check` from CI or a fresh
// clone would otherwise fail with "folder not found" before the
// dev could even run `cargo make web-build`.
//
// To keep `cargo build` from blocking on an explicit SPA build,
// this build script seeds an `index.html` placeholder that points
// the maintainer at the real command. The placeholder is overwritten
// the moment `bun run build` runs.
//
// Production releases (release.yml) build the real SPA before
// invoking cargo, so the placeholder is never actually embedded
// in a tagged release binary.

use std::fs;
use std::path::Path;

const PLACEHOLDER_HTML: &str = r#"<!doctype html>
<html lang="ja">
  <head><meta charset="utf-8" /><title>kotonoha</title></head>
  <body style="font-family: sans-serif; padding: 2rem; max-width: 40rem; margin: 0 auto;">
    <h1>kotonoha — SPA not built</h1>
    <p>
      This binary was compiled without the React frontend bundle.
      Run <code>cargo make web-build</code> from the workspace root
      (or <code>bun install &amp;&amp; bun run build</code> inside
      <code>backend/crates/kotonoha-server/web/</code>) and rebuild.
    </p>
    <p>
      The API is still live at
      <a href="/api/info">/api/info</a> and the WebSocket at
      <code>/ws/chat</code> — only the UI is missing.
    </p>
  </body>
</html>
"#;

fn main() {
    println!("cargo:rerun-if-changed=web/dist");
    let dist = Path::new("web/dist");
    let index = dist.join("index.html");
    if !index.exists() {
        fs::create_dir_all(dist).expect("create web/dist");
        fs::write(&index, PLACEHOLDER_HTML).expect("write placeholder index.html");
        println!(
            "cargo:warning=web/dist/ was empty — wrote a placeholder index.html. Run `cargo make web-build` to embed the real SPA."
        );
    }
}
