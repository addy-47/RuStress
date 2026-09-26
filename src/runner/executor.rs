use std::time::Instant;

use reqwest::Response;
use tokio::process::Command;

use crate::core::config::Config;
use crate::core::constants::MAX_CAPTURED_BODY_BYTES;
use crate::core::result::ExperimentResult;
use crate::runner::request::PreparedRequest;
use crate::runner::stats::RunStats;
use crate::templates::{TemplateContext, TemplateEngine};

/// Appended to a captured body that was cut short at the capture cap, so a
/// truncated diagnostic is never mistaken for a complete one.
///
/// Public so a report consumer — or a test — can detect truncation from the
/// constant rather than by matching a string literal that will drift.
pub const TRUNCATION_MARKER: &str = "\n...[truncated]";

/// Outcome of a single HTTP exchange, with body capture already bounded.
struct HttpOutcome {
    status: u16,
    bytes: u64,
    captured_body: Option<String>,
    error: Option<String>,
}

/// Execute a single HTTP request and fold the outcome into `stats`.
pub async fn execute_http(
    client: &reqwest::Client,
    prepared: &PreparedRequest,
    engine: &TemplateEngine,
    ctx: &TemplateContext,
    scheduled: Instant,
    stats: &RunStats,
) {
    let actual_start = Instant::now();
    let queue_wait = actual_start.saturating_duration_since(scheduled);

    let request = match prepared.build(client, engine, ctx) {
        Ok(request) => request,
        Err(message) => {
            stats.record(failed_outcome(
                actual_start,
                scheduled,
                ctx,
                0,
                message,
                None,
            ));
            return;
        }
    };

    let outcome = match request.send().await {
        Ok(response) => read_response(response).await,
        Err(e) => HttpOutcome {
            status: 0,
            bytes: 0,
            captured_body: None,
            error: Some(clean_error_msg(&e.to_string())),
        },
    };

    // A 2xx status alone is not a success. If the body stream failed, the
    // response was unusable, and counting it as a success lets a run report
    // 100% success against a target returning nothing but broken payloads
    // while `error_counts` disagrees with the headline number.
    let success = outcome.error.is_none() && (200..300).contains(&outcome.status);
    stats.record(ExperimentResult {
        timestamp: chrono::Utc::now(),
        latency: Instant::now().saturating_duration_since(scheduled),
        service_time: Instant::now().saturating_duration_since(actual_start),
        queue_wait,
        status: outcome.status,
        success,
        bytes: outcome.bytes as i64,
        user_id: ctx.user_id.clone(),
        query: prepared.label.clone(),
        error: outcome.error,
        response_body: outcome.captured_body,
    });
}

/// Drain a response body to EOF, counting bytes and capturing a bounded prefix.
///
/// The body is never fully materialised: bytes are counted as they stream past
/// so the throughput metric stays correct for chunked responses, which report
/// no `content-length`, and only the first [`MAX_CAPTURED_BODY_BYTES`] are
/// retained because a load generator cannot trust the size of a body from a
/// target it does not control.
///
/// Draining to EOF is deliberate. Abandoning the body early would forfeit
/// connection reuse, so a target serving large bodies would pay a fresh
/// TCP+TLS handshake on every request — and that handshake would land inside
/// the measured service time, corrupting the result.
async fn read_response(mut response: Response) -> HttpOutcome {
    let status = response.status().as_u16();
    let mut bytes = 0u64;
    let mut captured: Vec<u8> = Vec::new();
    let mut truncated = false;
    let mut stream_error = None;

    loop {
        match response.chunk().await {
            Ok(Some(chunk)) => {
                bytes += chunk.len() as u64;
                if captured.len() < MAX_CAPTURED_BODY_BYTES {
                    let room = MAX_CAPTURED_BODY_BYTES - captured.len();
                    let take = room.min(chunk.len());
                    captured.extend_from_slice(&chunk[..take]);
                    if take < chunk.len() {
                        truncated = true;
                    }
                } else {
                    truncated = true;
                }
            }
            Ok(None) => break,
            Err(e) => {
                stream_error = Some(clean_error_msg(&e.to_string()));
                break;
            }
        }
    }

    let captured_body = if status >= 400 && !captured.is_empty() {
        let mut text = String::from_utf8_lossy(&captured).into_owned();
        if truncated {
            text.push_str(TRUNCATION_MARKER);
        }
        Some(text)
    } else {
        None
    };

    HttpOutcome {
        status,
        bytes,
        captured_body,
        error: stream_error,
    }
}

/// Execute a single shell command via `sh -c` and fold the outcome into `stats`.
pub async fn execute_script(
    cfg: &Config,
    engine: &TemplateEngine,
    ctx: &TemplateContext,
    scheduled: Instant,
    stats: &RunStats,
) {
    let actual_start = Instant::now();
    let queue_wait = actual_start.saturating_duration_since(scheduled);

    let raw = cfg.command.as_deref().unwrap_or_default();
    let command = if raw.contains("{{") {
        engine
            .execute_str(raw, ctx)
            .unwrap_or_else(|_| raw.to_string())
    } else {
        raw.to_string()
    };

    let output = match Command::new("sh").arg("-c").arg(&command).output().await {
        Ok(output) => output,
        Err(e) => {
            stats.record(failed_outcome(
                actual_start,
                scheduled,
                ctx,
                0,
                e.to_string(),
                Some("shell".to_string()),
            ));
            return;
        }
    };

    let success = output.status.success();
    let status = if success {
        200
    } else {
        output.status.code().unwrap_or(500) as u16
    };
    let stderr = String::from_utf8_lossy(&output.stderr);
    let error = (!success).then(|| clean_error_msg(&stderr));
    let captured = (!success)
        .then(|| truncate_capture(&stderr))
        .filter(|s| !s.is_empty());

    stats.record(ExperimentResult {
        timestamp: chrono::Utc::now(),
        latency: Instant::now().saturating_duration_since(scheduled),
        service_time: Instant::now().saturating_duration_since(actual_start),
        queue_wait,
        status,
        success,
        bytes: output.stdout.len() as i64,
        user_id: ctx.user_id.clone(),
        query: "script".to_string(),
        error,
        response_body: captured,
    });
}

/// Build a failed result for a request that never reached the wire.
fn failed_outcome(
    actual_start: Instant,
    scheduled: Instant,
    ctx: &TemplateContext,
    status: u16,
    error: String,
    query: Option<String>,
) -> ExperimentResult {
    ExperimentResult {
        timestamp: chrono::Utc::now(),
        latency: Instant::now().saturating_duration_since(scheduled),
        service_time: Instant::now().saturating_duration_since(actual_start),
        queue_wait: actual_start.saturating_duration_since(scheduled),
        status,
        success: false,
        bytes: 0,
        user_id: ctx.user_id.clone(),
        query: query.unwrap_or_else(|| "custom".to_string()),
        error: Some(error),
        response_body: None,
    }
}

/// Truncate a diagnostic string to the capture cap.
fn truncate_capture(text: &str) -> String {
    text.chars().take(MAX_CAPTURED_BODY_BYTES).collect()
}

/// Strip redundant transport prefixes from a reqwest error message.
fn clean_error_msg(message: &str) -> String {
    let interesting =
        message.contains("dial") || message.contains("timeout") || message.contains("connect");
    if interesting {
        if let Some(idx) = message.rfind(": ") {
            return message[idx + 2..].to_string();
        }
    }
    message.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_error_msg_strips_transport_prefix() {
        assert_eq!(
            clean_error_msg("error sending request for url (http://x/): connection refused"),
            "connection refused"
        );
    }

    #[test]
    fn clean_error_msg_preserves_uninteresting_message() {
        let msg = "builder error: invalid header";
        assert_eq!(clean_error_msg(msg), msg);
    }

    #[test]
    fn truncate_capture_bounds_length() {
        let huge = "a".repeat(MAX_CAPTURED_BODY_BYTES * 4);
        let capped = truncate_capture(&huge);
        assert_eq!(capped.chars().count(), MAX_CAPTURED_BODY_BYTES);
    }

    #[test]
    fn truncate_capture_passes_short_text_through() {
        assert_eq!(truncate_capture("boom"), "boom");
    }

    #[test]
    fn truncate_capture_respects_char_boundaries() {
        let multibyte = "é".repeat(MAX_CAPTURED_BODY_BYTES);
        let capped = truncate_capture(&multibyte);
        assert_eq!(capped.chars().count(), MAX_CAPTURED_BODY_BYTES);
    }

    #[test]
    fn truncate_capture_at_exactly_the_cap_is_unchanged() {
        let text = "a".repeat(MAX_CAPTURED_BODY_BYTES);
        assert_eq!(
            truncate_capture(&text),
            text,
            "a capture of exactly the cap is complete and must pass through byte \
             for byte, marker or not"
        );
    }

    #[test]
    fn truncate_capture_never_splits_a_multibyte_character() {
        let multibyte = "é".repeat(MAX_CAPTURED_BODY_BYTES + 8);
        let capped = truncate_capture(&multibyte);
        assert_eq!(capped.chars().count(), MAX_CAPTURED_BODY_BYTES);
        assert!(
            std::str::from_utf8(capped.as_bytes()).is_ok(),
            "a cut diagnostic must remain valid UTF-8; a split character is written \
             into a CSV and JSON report as a replacement glyph"
        );
    }

    #[test]
    fn a_truncated_capture_is_one_char_shorter_not_one_byte_shorter() {
        let text = "a".repeat(MAX_CAPTURED_BODY_BYTES + 1);
        let capped = truncate_capture(&text);
        assert_eq!(capped.chars().count(), MAX_CAPTURED_BODY_BYTES);
        assert_eq!(capped.len(), MAX_CAPTURED_BODY_BYTES);
    }
}
