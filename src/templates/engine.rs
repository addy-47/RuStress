use minijinja::Environment;
use parking_lot::RwLock;
use rand::Rng;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use super::cache::FileCache;
use super::context::TemplateContext;

/// Pre-parsed template ready for execution.
pub type ParsedTemplate = Arc<String>;

/// Template engine with custom functions and file caching.
///
/// # Compilation happens once
///
/// A minijinja `Environment` is built once, in `new`, and every template is
/// compiled into it exactly once via `add_template_owned`. The engine is
/// `Arc`-shared and the environment sits behind a `RwLock`, so compiling is a
/// write-lock acquisition and rendering is a read-lock plus a compiled-template
/// lookup.
///
/// This was previously the single most expensive mistake available in the
/// codebase: `execute_str` constructed a `Environment`, registered six
/// functions and re-parsed the template on *every* call, and it is the path
/// the executor takes per request for a templated URL, header or body.
/// Rendering a templated request cost a full parse. `benches/hot_path_bench.rs`
/// reports allocs/request so a reintroduced parse is visible.
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
    /// The single environment every template is compiled into.
    env: RwLock<Environment<'static>>,
    /// Processed template source -> the name it is registered under.
    ///
    /// Keyed by source so the hot path, which carries template text rather than
    /// a handle, still resolves to the already-compiled template.
    compiled: RwLock<HashMap<String, String>>,
    /// Source templates registered by `parse`, by caller-chosen name.
    templates: RwLock<HashMap<String, ParsedTemplate>>,
    /// Disambiguates generated template names.
    next_id: AtomicU64,
}

impl TemplateEngine {
    pub fn new() -> Self {
        let file_cache = Arc::new(FileCache::new());
        let mut env = Environment::new();
        Self::register_functions(&mut env, Arc::clone(&file_cache));

        Self {
            file_cache,
            env: RwLock::new(env),
            compiled: RwLock::new(HashMap::new()),
            templates: RwLock::new(HashMap::new()),
            next_id: AtomicU64::new(0),
        }
    }

    /// Compile `source` into the environment if it is not already compiled, and
    /// return the name it is registered under.
    fn compile_once(&self, source: &str) -> anyhow::Result<String> {
        if let Some(name) = self.compiled.read().get(source) {
            return Ok(name.clone());
        }

        // The environment is `Environment<'static>`, so both the name and the
        // source must be owned -- a borrowed `&str` cannot be stored in it.
        let name = format!(
            "__compiled_{}",
            self.next_id.fetch_add(1, Ordering::Relaxed)
        );

        self.env
            .write()
            .add_template_owned(name.clone(), source.to_string())
            .map_err(|e| anyhow::anyhow!("failed to parse template: {e}"))?;

        self.compiled
            .write()
            .insert(source.to_string(), name.clone());

        Ok(name)
    }

    /// Render an already-compiled template.
    fn render_named(&self, name: &str, ctx: &TemplateContext) -> anyhow::Result<String> {
        let env = self.env.read();
        let tmpl = env
            .get_template(name)
            .map_err(|e| anyhow::anyhow!("failed to get template: {e}"))?;

        tmpl.render(minijinja::context! {
            user_id => &ctx.user_id,
        })
        .map_err(|e| anyhow::anyhow!("failed to render template: {e}"))
    }

    /// Parse and store a template by name.
    pub fn parse(&self, name: &str, text: &str) -> anyhow::Result<ParsedTemplate> {
        // Preprocess: convert Go-style syntax to minijinja function calls.
        let processed = Self::preprocess(text);

        self.env
            .write()
            .add_template_owned(name.to_string(), processed.clone())
            .map_err(|e| anyhow::anyhow!("failed to parse template '{name}': {e}"))?;

        let tpl = Arc::new(processed);
        self.templates
            .write()
            .insert(name.to_string(), Arc::clone(&tpl));
        Ok(tpl)
    }

    /// Execute a pre-parsed template with the given context.
    /// Note: only `user_id` is passed as a variable; `uuid` is always produced
    /// by the `uuid()` function, so passing it here would shadow the function
    /// with a string.
    pub fn execute(
        &self,
        template: &ParsedTemplate,
        ctx: &TemplateContext,
    ) -> anyhow::Result<String> {
        let name = self.compile_once(template.as_str())?;
        self.render_named(&name, ctx)
    }

    /// Execute a template string directly (convenience method).
    /// Render a template given as text.
    ///
    /// This is the path the executor takes per templated component, so it must
    /// not compile. Identical text resolves to the same already-compiled
    /// template after the first call.
    pub fn execute_str(&self, text: &str, ctx: &TemplateContext) -> anyhow::Result<String> {
        let processed = Self::preprocess(text);
        let name = self.compile_once(&processed)?;
        self.render_named(&name, ctx)
    }

    /// How many distinct templates have been compiled into the environment.
    ///
    /// A test seam: a per-request recompile shows up here as a count that
    /// grows with the number of renders.
    pub fn compiled_template_count(&self) -> usize {
        self.compiled.read().len()
    }

    /// Get the file cache for testing.
    pub fn file_cache(&self) -> &Arc<FileCache> {
        &self.file_cache
    }

    /// Add custom functions to a minijinja environment.
    ///
    /// Called exactly once, from `new`. Registering these per request was part
    /// of the per-request `Environment` cost this engine no longer pays.
    fn register_functions(env: &mut Environment<'static>, file_cache: Arc<FileCache>) {
        let file_cache = Arc::clone(&file_cache);

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
