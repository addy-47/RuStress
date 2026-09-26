use minijinja::Environment;
use parking_lot::RwLock;
use rand::Rng;
use std::collections::HashMap;
use std::sync::Arc;

use super::cache::FileCache;
use super::context::TemplateContext;

/// Pre-parsed template ready for execution.
pub type ParsedTemplate = Arc<String>;

/// Template engine with custom functions and file caching.
///
/// Supports the following custom functions:
/// - `{{ random_int(min, max) }}` — random integer in range [min, max)
/// - `{{ uuid() }}` — generates a new UUID v4
/// - `{{ user_id }}` — the current user ID, provided as a context variable
/// - `{{ request_uuid() }}` — returns the current request UUID
/// - `{{ random_choice(a, b, c) }}` — picks one value randomly
/// - `{{ random_line(path) }}` — picks a random line from a file
/// - `{{ read_file(path) }}` — reads entire file content
pub struct TemplateEngine {
    file_cache: Arc<FileCache>,
    /// Cache of pre-parsed templates by name.
    templates: RwLock<HashMap<String, ParsedTemplate>>,
}

impl TemplateEngine {
    pub fn new() -> Self {
        let file_cache = Arc::new(FileCache::new());

        Self {
            file_cache,
            templates: RwLock::new(HashMap::new()),
        }
    }

    /// Parse and store a template by name.
    pub fn parse(&self, name: &str, text: &str) -> anyhow::Result<ParsedTemplate> {
        // Preprocess: convert Go-style syntax to minijinja function calls.
        let processed = Self::preprocess(text);

        // Validate by parsing.
        let mut env = Environment::new();
        self.add_functions_to_env(&mut env);

        env.add_template(name, &processed)
            .map_err(|e| anyhow::anyhow!("failed to parse template '{}': {}", name, e))?;

        let tpl = Arc::new(processed);
        self.templates
            .write()
            .insert(name.to_string(), Arc::clone(&tpl));
        Ok(tpl)
    }

    /// Execute a pre-parsed template with the given context.
    pub fn execute(
        &self,
        template: &ParsedTemplate,
        ctx: &TemplateContext,
    ) -> anyhow::Result<String> {
        let mut env = Environment::new();
        self.add_functions_to_env(&mut env);

        env.add_template("__exec__", template.as_str())
            .map_err(|e| anyhow::anyhow!("failed to add template: {}", e))?;

        let tmpl = env
            .get_template("__exec__")
            .map_err(|e| anyhow::anyhow!("failed to get template: {}", e))?;

        // Note: only pass user_id as a variable; uuid is always generated via uuid() function
        // to avoid shadowing the function with a string variable.
        let result = tmpl
            .render(minijinja::context! {
                user_id => &ctx.user_id,
            })
            .map_err(|e| anyhow::anyhow!("failed to render template: {}", e))?;

        Ok(result)
    }

    /// Execute a template string directly (convenience method).
    pub fn execute_str(&self, text: &str, ctx: &TemplateContext) -> anyhow::Result<String> {
        let processed = Self::preprocess(text);
        let mut env = Environment::new();
        self.add_functions_to_env(&mut env);

        env.add_template("__exec__", &processed)
            .map_err(|e| anyhow::anyhow!("failed to parse template: {}", e))?;

        let tmpl = env
            .get_template("__exec__")
            .map_err(|e| anyhow::anyhow!("failed to get template: {}", e))?;

        let result = tmpl
            .render(minijinja::context! {
                user_id => &ctx.user_id,
            })
            .map_err(|e| anyhow::anyhow!("failed to render template: {}", e))?;

        Ok(result)
    }

    /// Get the file cache for testing.
    pub fn file_cache(&self) -> &Arc<FileCache> {
        &self.file_cache
    }

    /// Add custom functions to a minijinja environment.
    fn add_functions_to_env(&self, env: &mut Environment<'_>) {
        let file_cache = Arc::clone(&self.file_cache);

        env.add_function("random_int", |min: i64, max: i64| -> i64 {
            if min >= max {
                min
            } else {
                rand::thread_rng().gen_range(min..max)
            }
        });

        env.add_function("uuid", || -> String { uuid::Uuid::new_v4().to_string() });

        env.add_function("random_choice", |values: Vec<String>| -> String {
            if values.is_empty() {
                String::new()
            } else {
                let idx = rand::thread_rng().gen_range(0..values.len());
                values[idx].clone()
            }
        });

        let fc1 = Arc::clone(&file_cache);
        env.add_function("random_line", move |path: String| -> String {
            match fc1.get_lines(&path) {
                Ok(lines) if !lines.is_empty() => {
                    let idx = rand::thread_rng().gen_range(0..lines.len());
                    lines[idx].clone()
                }
                _ => String::new(),
            }
        });

        let fc2 = Arc::clone(&file_cache);
        env.add_function("read_file", move |path: String| -> String {
            fc2.get_raw(&path).unwrap_or_default()
        });

        // printf-style formatting using minijinja's built-in format filter
        // (not needed as a custom function — minijinja supports {{ "%s"|format(val) }})
    }

    /// Preprocess template text to support Go-style syntax.
    ///
    /// Converts:
    /// - `{{userID}}` → `{{ user_id }}`
    /// - `{{uuid}}` / `{{requestID}}` → `{{ uuid() }}`
    fn preprocess(input: &str) -> String {
        let mut output = input.to_string();
        output = output.replace("{{userID}}", "{{ user_id }}");
        output = output.replace("{{uuid}}", "{{ uuid() }}");
        output = output.replace("{{requestID}}", "{{ uuid() }}");
        output
    }
}

impl Default for TemplateEngine {
    fn default() -> Self {
        Self::new()
    }
}
