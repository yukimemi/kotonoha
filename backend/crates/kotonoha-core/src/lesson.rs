use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;

use crate::config::render_toml;

#[derive(Debug, Clone, Deserialize)]
pub struct Lesson {
    #[serde(default)]
    pub vars: BTreeMap<String, toml::Value>,
    pub system_prompt: String,
}

impl Lesson {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let rendered = render_toml(path)?;
        let lesson: Lesson = toml::from_str(&rendered)?;
        Ok(lesson)
    }
}
