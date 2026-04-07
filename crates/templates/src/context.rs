/// Data passed to template execution context.
#[derive(Debug, Clone)]
pub struct TemplateContext {
    /// Stable virtual user identifier.
    pub user_id: String,
    /// Fresh UUID per request.
    pub uuid: String,
}

impl TemplateContext {
    pub fn new(user_id: String, uuid: String) -> Self {
        Self { user_id, uuid }
    }
}

impl Default for TemplateContext {
    fn default() -> Self {
        Self {
            user_id: String::new(),
            uuid: String::new(),
        }
    }
}
