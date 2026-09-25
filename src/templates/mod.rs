//! Request templating: per-request value injection and file-backed data sources.

pub mod cache;
pub mod context;
pub mod engine;

pub use cache::FileCache;
pub use context::TemplateContext;
pub use engine::TemplateEngine;
