/// Data passed to template execution context.
#[derive(Debug, Clone, Default)]
pub struct TemplateContext {
    /// Stable virtual user identifier, available to templates as `user_id`.
    pub user_id: String,
}

impl TemplateContext {
    pub fn new(user_id: String) -> Self {
        Self { user_id }
    }
}
