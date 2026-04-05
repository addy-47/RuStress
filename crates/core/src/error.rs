use thiserror::Error;

/// Custom error types for Rustress.
#[derive(Error, Debug)]
pub enum RustressError {
    #[error("template error: {0}")]
    TemplateError(String),

    #[error("network error: {0}")]
    NetworkError(String),

    #[error("script execution error: {0}")]
    ScriptError(String),

    #[error("export error: {0}")]
    ExportError(String),

    #[error("configuration error: {0}")]
    ConfigError(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}
