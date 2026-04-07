use std::collections::HashMap;

use crate::templates::QueryTemplate;

pub struct QueryParams {
    pub template_id: String,
    pub parameters: HashMap<String, String>,
}

/// Generate a SQL query by finding the template in the provided slice and rendering it.
pub fn generate_query(params: QueryParams, templates: &[QueryTemplate]) -> Result<String, String> {
    let template = templates
        .iter()
        .find(|t| t.id == params.template_id)
        .ok_or_else(|| format!("Template '{}' not found", params.template_id))?;

    template.render(&params.parameters)
}
