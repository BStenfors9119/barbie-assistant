use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

// ── Preference types (live here to avoid circular imports) ────────────────────

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub enum ThemeColor {
    #[default]
    Blue,
    Gray,
    Green,
}

impl fmt::Display for ThemeColor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ThemeColor::Blue => write!(f, "Blue"),
            ThemeColor::Gray => write!(f, "Gray"),
            ThemeColor::Green => write!(f, "Green"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub enum ThemeMode {
    #[default]
    Light,
    Dark,
}

impl fmt::Display for ThemeMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ThemeMode::Light => write!(f, "Light"),
            ThemeMode::Dark => write!(f, "Dark"),
        }
    }
}

// ── Persisted settings ────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
pub struct Settings {
    pub font_size: u16,
    pub theme_color: ThemeColor,
    pub theme_mode: ThemeMode,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            font_size: 14,
            theme_color: ThemeColor::default(),
            theme_mode: ThemeMode::default(),
        }
    }
}

impl Settings {
    pub fn load() -> Self {
        let path = config_path();
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let path = config_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(path, json);
        }
    }
}

fn config_path() -> PathBuf {
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
    base.join("barbie-assistant").join("settings.json")
}
