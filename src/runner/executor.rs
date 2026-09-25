use reqwest::Client;
use rustress_core::config::Config;
use rustress_core::result::ExperimentResult;
use rustress_templates::TemplateContext;
use rustress_templates::TemplateEngine;
use std::time::Instant;
use tokio::process::Command;

use crate::stats::RunStats;

/// Execute a single HTTP request.
pub async fn execute_http(
    client: &Client,
    cfg: &Config,
    engine: &TemplateEngine,
    ctx: &TemplateContext,
    scheduled: Instant,
    stats: &RunStats,
) {
    let actual_start = Instant::now();
    let queue_wait = actual_start.saturating_duration_since(scheduled);

    stats.inc_inflight();

    let method = if cfg.method.is_empty() { "GET" } else { cfg.method.as_str() };

    // Build URL
    let url = if cfg.url.contains("{{") {
        engine.execute_str(&cfg.url, ctx).unwrap_or_else(|_| cfg.url.clone())
    } else {
        cfg.url.clone()
    };

    // Build body
    let body_str = if let Some(ref body) = cfg.body {
        let body_text = if body.starts_with('@') {
            let fname = body.strip_prefix('@').unwrap();
            format!(r#"{{{{ read_file("{}") }}}}"#, fname)
        } else {
            body.clone()
        };
        engine.execute_str(&body_text, ctx).unwrap_or_else(|_| body.clone())
    } else {
        String::new()
    };

    // Build request
    let mut req = client.request(
        reqwest::Method::from_bytes(method.as_bytes()).unwrap_or(reqwest::Method::GET),
        &url,
    );

    // Add headers
    let has_content_type = cfg.headers.keys().any(|k| k.to_lowercase() == "content-type");
    for (k, v) in &cfg.headers {
        let val = if v.contains("{{") {
            engine.execute_str(v, ctx).unwrap_or_else(|_| v.clone())
        } else {
            v.clone()
        };
        req = req.header(k, val);
    }
    if !has_content_type && !body_str.is_empty() {
        req = req.header("Content-Type", "application/json");
    }

    if !body_str.is_empty() {
        req = req.body(body_str);
    }

    // Execute
    let (status, bytes_len, resp_body, err_str) = match req.send().await {
        Ok(resp) => {
            let status = resp.status().as_u16();
            let bytes_len = resp.content_length().unwrap_or(0) as i64;

            if status >= 400 {
                match resp.text().await {
                    Ok(body) => (status, bytes_len, Some(body), None),
                    Err(e) => (status, bytes_len, None, Some(e.to_string())),
                }
            } else {
                // Consume body for connection reuse.
                let _ = resp.bytes().await;
                (status, bytes_len, None, None)
            }
        }
        Err(e) => {
            let err_str = clean_error_msg(&e.to_string());
            (0, 0, None, Some(err_str))
        }
    };

    let end_time = Instant::now();
    let service_time = end_time.duration_since(actual_start);
    let total_latency = end_time.duration_since(scheduled);

    let success = status >= 200 && status < 300;

    let result = ExperimentResult {
        timestamp: chrono::Utc::now(),
        latency: total_latency,
        service_time,
        queue_wait,
        status,
        success,
        bytes: bytes_len,
        user_id: ctx.user_id.clone(),
        query: "custom".into(),
        error: err_str.clone(),
        response_body: resp_body,
    };

    stats.record(&result);
    stats.dec_inflight();
}

/// Execute a single shell command.
pub async fn execute_script(
    cfg: &Config,
    engine: &TemplateEngine,
    ctx: &TemplateContext,
    scheduled: Instant,
    stats: &RunStats,
) {
    let actual_start = Instant::now();
    let queue_wait = actual_start.saturating_duration_since(scheduled);

    stats.inc_inflight();

    // Template the command
    let cmd_str = if cfg.command.as_ref().map_or(false, |c| c.contains("{{")) {
        let cmd = cfg.command.as_deref().unwrap_or("");
        engine.execute_str(cmd, ctx).unwrap_or_else(|_| cmd.to_string())
    } else {
        cfg.command.clone().unwrap_or_default()
    };

    // Execute via sh -c
    let output = match Command::new("sh").arg("-c").arg(&cmd_str).output().await {
        Ok(out) => out,
        Err(e) => {
            let service_time = Instant::now().duration_since(actual_start);
            let total_latency = Instant::now().duration_since(scheduled);

            let result = ExperimentResult {
                timestamp: chrono::Utc::now(),
                latency: total_latency,
                service_time,
                queue_wait,
                status: 500,
                success: false,
                bytes: 0,
                user_id: ctx.user_id.clone(),
                query: "custom".into(),
                error: Some(e.to_string()),
                response_body: None,
            };
            stats.record(&result);
            stats.dec_inflight();
            return;
        }
    };

    let end_time = Instant::now();
    let service_time = end_time.duration_since(actual_start);
    let total_latency = end_time.duration_since(scheduled);

    let exit_code = output.status.code().unwrap_or(500) as u16;
    let success = output.status.success();
    let status = if success { 200 } else { exit_code };

    let result = ExperimentResult {
        timestamp: chrono::Utc::now(),
        latency: total_latency,
        service_time,
        queue_wait,
        status,
        success,
        bytes: output.stdout.len() as i64,
        user_id: ctx.user_id.clone(),
        query: "custom".into(),
        error: if !success {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Some(clean_error_msg(&stderr))
        } else {
            None
        },
        response_body: if !success {
            Some(String::from_utf8_lossy(&output.stderr).into_owned())
        } else {
            None
        },
    };

    stats.record(&result);
    stats.dec_inflight();
}

/// Clean up error messages to remove redundant prefixes.
fn clean_error_msg(msg: &str) -> String {
    if let Some(idx) = msg.rfind(": ") {
        if msg.contains("dial") || msg.contains("timeout") || msg.contains("connect") {
            return msg[idx + 2..].to_string();
        }
    }
    msg.to_string()
}
