use std::path::PathBuf;

fn main() -> anyhow::Result<()> {
    let cfg_path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("./configs/kotonoha.toml"));
    let cfg = kotonoha_core::Config::load(&cfg_path)?;
    for name in cfg.lesson.keys() {
        match cfg.load_lesson(name) {
            Ok(l) => println!(
                "OK  {name}: prompt={} chars, vars={}",
                l.system_prompt.len(),
                l.vars.len()
            ),
            Err(e) => println!("ERR {name}: {e:#}"),
        }
    }
    Ok(())
}
