use std::collections::HashMap;
use std::path::PathBuf;

use serde::Deserialize;

#[derive(Debug, Clone)]
pub struct QueryTemplate {
    pub id: String,
    pub group: Option<String>,
    pub name: String,
    pub description: String,
    pub sql: String,
}

// ── Raw JSON shape ────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct RawTemplate {
    id: String,
    #[serde(default)]
    group: Option<String>,
    name: String,
    #[serde(default)]
    description: String,
    sql: String,
}

impl From<RawTemplate> for QueryTemplate {
    fn from(r: RawTemplate) -> Self {
        Self {
            id: r.id,
            group: r.group,
            name: r.name,
            description: r.description,
            sql: r.sql,
        }
    }
}

// ── Built-in templates ────────────────────────────────────────────────────────

impl QueryTemplate {
    pub fn builtin() -> Vec<QueryTemplate> {
        vec![
            QueryTemplate {
                id: "travel_requests_by_employee".to_string(),
                group: Some("Built-in".to_string()),
                name: "Travel Requests by Employee".to_string(),
                description: "All travel requests for a given employee (PERNR)".to_string(),
                sql: "SELECT * FROM PTRV_HEAD WHERE PERNR = '{employee_id}'".to_string(),
            },
            QueryTemplate {
                id: "travel_expenses_by_date".to_string(),
                group: Some("Built-in".to_string()),
                name: "Travel Expenses by Date Range".to_string(),
                description: "Expenses for a request within a date range".to_string(),
                sql: "SELECT * FROM PTRV_PERIO WHERE REINR = '{request_id}' AND BUDAT BETWEEN '{start_date}' AND '{end_date}'".to_string(),
            },
            QueryTemplate {
                id: "open_travel_requests".to_string(),
                group: Some("Built-in".to_string()),
                name: "Open Travel Requests".to_string(),
                description: "All travel requests with open/pending status".to_string(),
                sql: "SELECT * FROM PTRV_HEAD WHERE REINR LIKE '{prefix}%' AND STATV = '10'".to_string(),
            },
        ]
    }

    // ── File-based templates ──────────────────────────────────────────────────

    /// Load all `.json` files from the `_templates` directory next to the
    /// executable (production) or the current working directory (cargo run).
    pub fn load_from_dir() -> Vec<QueryTemplate> {
        let dir = templates_dir();
        let mut out = Vec::new();

        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => return out,
        };

        // Collect and sort paths so loading order is deterministic.
        let mut paths: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().map_or(false, |e| e == "json"))
            .collect();
        paths.sort();

        for path in paths {
            if let Ok(content) = std::fs::read_to_string(&path) {
                if let Ok(raw) = serde_json::from_str::<Vec<RawTemplate>>(&content) {
                    out.extend(raw.into_iter().map(QueryTemplate::from));
                }
            }
        }

        out
    }

    // ── Param helpers ─────────────────────────────────────────────────────────

    /// Extract placeholder keys from the SQL (e.g. `{employee_id}` → `"employee_id"`).
    pub fn param_keys(&self) -> Vec<String> {
        let mut keys = Vec::new();
        let mut chars = self.sql.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '{' {
                let key: String = chars.by_ref().take_while(|&ch| ch != '}').collect();
                if !key.is_empty() {
                    keys.push(key);
                }
            }
        }
        keys
    }

    /// Substitute placeholders with provided values.
    pub fn render(&self, params: &HashMap<String, String>) -> Result<String, String> {
        let mut sql = self.sql.clone();
        for (key, value) in params {
            sql = sql.replace(&format!("{{{}}}", key), value);
        }
        if sql.contains('{') {
            let missing: Vec<String> = self
                .param_keys()
                .into_iter()
                .filter(|k| !params.contains_key(k))
                .collect();
            return Err(format!("Missing parameters: {}", missing.join(", ")));
        }
        Ok(sql)
    }
}

// ── Path resolution ───────────────────────────────────────────────────────────

fn templates_dir() -> PathBuf {
    // Production: look next to the executable.
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let p = dir.join("_templates");
            if p.is_dir() {
                return p;
            }
        }
    }
    // Development (cargo run): look in the working directory.
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("_templates")
}
