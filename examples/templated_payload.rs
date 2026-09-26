//! ============================================================================
//! templated_payload.rs — per-request value injection
//! ============================================================================
//! Category     : Utility Tool
//! Component    : templates::TemplateEngine, runner::request::PreparedRequest
//! Prerequisites: none
//! Execution    : cargo run --release --example templated_payload
//! Metrics      : request count and confirmation that each body was unique
//!
//! Template directives in the URL, headers and body are rendered per request.
//! The engine compiles each distinct template exactly once and renders it many
//! times; building a template engine per request is the single most expensive
//! mistake available in this codebase, and it used to happen on this path.
//! ============================================================================

#[path = "common/mod.rs"]
mod shared;

use rustress::core::Config;
use rustress::templates::{TemplateContext, TemplateEngine};
use shared::{report, run_to_completion, start_target};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let base = start_target().await?;
    println!("target: {base}/fast");

    // Show the rendering directly, so the output is inspectable without
    // capturing traffic server-side.
    let engine = TemplateEngine::new();
    let ctx = TemplateContext::new("user-42".to_string());
    let rendered = engine.execute_str(r#"{"id": {{ user_id }}, "n": {{ uuid() }}}"#, &ctx)?;
    println!("rendered body: {rendered}");

    let cfg = Config {
        url: format!("{base}/fast?who={{{{ user_id }}}}&req={{{{ uuid() }}}}"),
        method: "POST".to_string(),
        body: Some(r#"{"user": "{{ user_id }}", "nonce": "{{ uuid() }}"}"#.to_string()),
        headers: indexmap::IndexMap::from([(
            "X-Request-Id".to_string(),
            "{{ uuid() }}".to_string(),
        )]),
        target_rps: 200,
        steady_dur_secs: 4,
        timeout_secs: 10,
        ..Default::default()
    };

    report(
        "templated URL, header and body at 200 RPS",
        &run_to_completion(cfg).await?,
    );
    Ok(())
}
