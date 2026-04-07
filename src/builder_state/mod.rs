use std::fmt;

use serde::{Deserialize, Serialize};

// ── Operator ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum WhereOperator {
    Eq,
    NotEq,
    Like,
    Gt,
    Lt,
    Gte,
    Lte,
}

impl Default for WhereOperator {
    fn default() -> Self {
        WhereOperator::Eq
    }
}

impl WhereOperator {
    pub fn all() -> Vec<WhereOperator> {
        vec![
            WhereOperator::Eq,
            WhereOperator::NotEq,
            WhereOperator::Like,
            WhereOperator::Gt,
            WhereOperator::Lt,
            WhereOperator::Gte,
            WhereOperator::Lte,
        ]
    }

    pub fn to_sql(&self) -> &'static str {
        match self {
            WhereOperator::Eq => "=",
            WhereOperator::NotEq => "<>",
            WhereOperator::Like => "LIKE",
            WhereOperator::Gt => ">",
            WhereOperator::Lt => "<",
            WhereOperator::Gte => ">=",
            WhereOperator::Lte => "<=",
        }
    }
}

impl fmt::Display for WhereOperator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_sql())
    }
}

// ── Condition ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
pub struct WhereCondition {
    pub field: Option<String>,
    pub operator: WhereOperator,
    /// Name of the query parameter that will fill this slot, e.g. "employee_id"
    /// → rendered as `'{employee_id}'` in the SQL template.
    pub param_name: String,
}

// ── Builder state ─────────────────────────────────────────────────────────────

#[derive(Debug, Default, Clone)]
pub struct BuilderState {
    pub template_name: String,
    pub selected_table: Option<String>,
    /// Columns in the SELECT list, ordered by selection.
    pub selected_columns: Vec<String>,
    pub conditions: Vec<WhereCondition>,
    pub save_error: Option<String>,
}

impl BuilderState {
    /// Generate the SQL template string, or `None` if not enough info yet.
    pub fn build_sql(&self) -> Option<String> {
        let table = self.selected_table.as_deref()?;
        if self.selected_columns.is_empty() {
            return None;
        }

        let cols = self.selected_columns.join(", ");
        let mut sql = format!("SELECT {cols}\nFROM {table}");

        let valid: Vec<&WhereCondition> = self
            .conditions
            .iter()
            .filter(|c| c.field.is_some() && !c.param_name.is_empty())
            .collect();

        if !valid.is_empty() {
            let clauses: Vec<String> = valid
                .iter()
                .map(|c| {
                    let field = c.field.as_deref().unwrap();
                    format!(
                        "{} {} '{{{}}}' ",
                        field,
                        c.operator.to_sql(),
                        c.param_name
                    )
                })
                .collect();
            sql.push_str("\nWHERE ");
            sql.push_str(&clauses.join("\n  AND "));
        }

        Some(sql)
    }

    /// Generate a stable identifier from the template name.
    pub fn build_template_id(&self) -> String {
        self.template_name
            .to_lowercase()
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { '_' })
            .collect()
    }

    /// Return `Ok(())` if the state is complete enough to save, or an error string.
    pub fn validate(&self) -> Result<(), String> {
        if self.template_name.trim().is_empty() {
            return Err("Template name is required.".to_string());
        }
        if self.selected_table.is_none() {
            return Err("Select a table first.".to_string());
        }
        if self.selected_columns.is_empty() {
            return Err("Select at least one field for SELECT.".to_string());
        }
        for (i, c) in self.conditions.iter().enumerate() {
            let has_field = c.field.is_some();
            let has_param = !c.param_name.is_empty();
            if has_field != has_param {
                return Err(format!(
                    "Condition {} is incomplete — set both the field and a parameter name.",
                    i + 1
                ));
            }
        }
        Ok(())
    }
}
