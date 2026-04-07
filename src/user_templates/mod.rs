use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserTemplate {
    pub id: String,
    pub name: String,
    pub description: String,
    pub sql: String,
}

impl UserTemplate {
    pub fn load_all() -> Vec<UserTemplate> {
        let path = templates_path();
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save_all(templates: &[UserTemplate]) {
        let path = templates_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(templates) {
            let _ = std::fs::write(path, json);
        }
    }
}

fn templates_path() -> PathBuf {
    let base: PathBuf = if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."))
    } else {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .map(|h| h.join(".config"))
            .unwrap_or_else(|| PathBuf::from("."))
    };
    base.join("barbie-assistant").join("templates.json")
}
