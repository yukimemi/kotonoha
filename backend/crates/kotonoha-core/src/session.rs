use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "role", content = "text", rename_all = "lowercase")]
pub enum Turn {
    Student(String),
    Teacher(String),
}

#[derive(Debug, Clone, Default)]
pub struct Session {
    pub turns: Vec<Turn>,
}

impl Session {
    pub fn push_student(&mut self, text: impl Into<String>) {
        self.turns.push(Turn::Student(text.into()));
    }

    pub fn push_teacher(&mut self, text: impl Into<String>) {
        self.turns.push(Turn::Teacher(text.into()));
    }

    pub fn reset(&mut self) {
        self.turns.clear();
    }
}
