//! ============================================================================
//! structural_safety_test — architectural invariants that cannot be observed
//!                          at runtime
//! ============================================================================
//! Category     : Integration Test (structural)
//! Component    : whole crate source; `Cargo.toml`; `tui::guard`
//! Prerequisites: none (reads the repository's own sources)
//! Execution    : cargo test --test structural_safety_test
//! Metrics      : number of offending call sites, by file
//! ============================================================================
//!
//! Three of the project's non-negotiable invariants are architectural rather
//! than behavioural and cannot be observed from a test binary:
//!
//! 1. Terminal takeover is confined to `tui::guard`. `TerminalGuard` cannot be
//!    exercised against a real TTY in CI, so the invariant is asserted by
//!    proving no other module reaches for the terminal.
//! 2. No profile sets `panic = "abort"`. Teardown lives in `Drop`; an abort
//!    skips destructors and reintroduces the broken-shell failure.
//! 3. No mock logic. A `MockHttpClient` in the test suite would make every
//!    other test in this file a comment rather than evidence.
//!
//! These assertions are grep-based on purpose. The property is about the shape
//! of the source, so the source is what must be read.

mod common;

use std::path::{Path, PathBuf};

/// Every `.rs` file under `src/`, sorted for a stable failure message.
fn production_sources() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        for entry in
            std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
        {
            let path = entry.expect("read dir entry").path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

/// Every call site of `needle` in `src/`, as `(file:line, text)`.
fn call_sites(needle: &str) -> Vec<(String, usize, String)> {
    let mut hits = Vec::new();
    for path in production_sources() {
        let text = std::fs::read_to_string(&path).expect("read source");
        for (i, line) in text.lines().enumerate() {
            if line.contains(needle) {
                hits.push((
                    path.strip_prefix(env!("CARGO_MANIFEST_DIR"))
                        .unwrap_or(&path)
                        .display()
                        .to_string(),
                    i + 1,
                    line.trim().to_string(),
                ));
            }
        }
    }
    hits
}

/// Terminal takeover must live behind `TerminalGuard` and nowhere else.
///
/// Catches any module that enables raw mode or the alternate screen directly.
/// The failure this prevents is not a failing test: it is a user whose shell
/// echoes nothing and ignores Ctrl-C after the tool exits.
#[test]
fn terminal_takeover_is_confined_to_the_tui_guard() {
    const FORBIDDEN: [&str; 3] = [
        "enable_raw_mode",
        "EnterAlternateScreen",
        "EnableMouseCapture",
    ];

    for needle in FORBIDDEN {
        let hits = call_sites(needle);
        let outside: Vec<_> = hits
            .iter()
            .filter(|(file, _, _)| file != "src/tui/guard.rs")
            .collect();
        assert!(
            outside.is_empty(),
            "`{needle}` must only be reached through tui::TerminalGuard, but it \
             also appears at {outside:?}"
        );
    }
}

/// Teardown must run from `Drop`, not as a trailing statement.
///
/// Catches a refactor that "tidies up" the guard by calling the restore sequence
/// at the end of the run function, which is exactly the shape that leaves the
/// terminal broken on any `?`, panic, or early return.
#[test]
fn the_terminal_guard_restores_from_drop() {
    let guard =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/tui/guard.rs"))
            .expect("read the guard");

    assert!(
        guard.contains("impl Drop for TerminalGuard"),
        "TerminalGuard must restore the terminal from Drop, or an early return or \
         panic skips teardown"
    );
    assert!(
        guard.contains("disable_raw_mode") && guard.contains("LeaveAlternateScreen"),
        "Drop must undo both halves of the takeover it performed"
    );
}

/// No profile may abort on panic.
///
/// `panic = "abort"` skips destructors, which is the mechanism the terminal
/// guard depends on. It must be absent from every profile, not just release.
#[test]
fn no_profile_aborts_on_panic() {
    let manifest =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"))
            .expect("read Cargo.toml");

    for (i, line) in manifest.lines().enumerate() {
        let trimmed = line.trim();
        assert!(
            !trimmed.starts_with("panic"),
            "Cargo.toml:{} sets `{trimmed}`; an abort skips Drop, and Drop is what \
             restores the user's terminal",
            i + 1
        );
    }
}

/// The production crate must contain no mock or fake collaborators.
///
/// A `MockHttpClient` substituted for the real client would make every
/// integration test in this suite a test of the mock. The reference target is a
/// real axum server precisely so that no such type is needed.
///
/// `Dummy` is deliberately absent from the prefix list: `dummy::DummyServer` is
/// a real HTTP server, which is the whole point of it.
#[test]
fn the_production_crate_defines_no_mock_or_fake_types() {
    const BANNED_PREFIXES: [&str; 3] = ["Mock", "Fake", "Stub"];
    // Written with `concat!` so this file does not itself contain the phrases it
    // searches for. The scanner below reads raw lines, string literals included.
    const BANNED_MARKERS: [&str; 3] = [
        concat!("trait ", "MockHttpClient"),
        "cfg(any(test, feature = \"test",
        "cfg(feature = \"mock",
    ];

    let mut offences = Vec::new();
    for path in production_sources() {
        let file = path
            .strip_prefix(env!("CARGO_MANIFEST_DIR"))
            .unwrap_or(&path)
            .display()
            .to_string();
        let text = std::fs::read_to_string(&path).expect("read source");
        for (i, line) in text.lines().enumerate() {
            let trimmed = line.trim();
            for marker in BANNED_MARKERS {
                if trimmed.contains(marker) {
                    offences.push(format!("{file}:{}: {marker}", i + 1));
                }
            }
            for prefix in BANNED_PREFIXES {
                if trimmed.starts_with("pub struct ")
                    || trimmed.starts_with("struct ")
                    || trimmed.starts_with("pub trait ")
                    || trimmed.starts_with("trait ")
                    || trimmed.starts_with("pub enum ")
                {
                    if let Some(name) = trimmed
                        .split_whitespace()
                        .nth(2)
                        .map(|n| n.trim_start_matches("mut ").to_string())
                    {
                        if name.starts_with(prefix) {
                            offences.push(format!("{file}:{}: {prefix}* type `{name}`", i + 1));
                        }
                    }
                }
            }
        }
    }

    assert!(
        offences.is_empty(),
        "the production crate must not define mock collaborators: {offences:#?}"
    );
}

/// The test suite must not introduce a mock either.
///
/// The Zero-Mock Rule covers `tests/` and `benches/`: a mock added to a test
/// makes that test meaningless while leaving the rest of the suite green.
#[test]
fn the_test_suite_defines_no_mock_collaborators() {
    const BANNED_PREFIXES: [&str; 3] = ["Mock", "Fake", "Stub"];

    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut offences = Vec::new();
    for dir in ["tests", "benches"] {
        let dir_path = root.join(dir);
        if !dir_path.exists() {
            continue;
        }
        let mut stack = vec![dir_path];
        while let Some(current) = stack.pop() {
            for entry in std::fs::read_dir(&current)
                .unwrap_or_else(|e| panic!("read {}: {e}", current.display()))
            {
                let path = entry.expect("read dir entry").path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    let file = path
                        .strip_prefix(env!("CARGO_MANIFEST_DIR"))
                        .unwrap_or(&path)
                        .display()
                        .to_string();
                    let text = std::fs::read_to_string(&path).expect("read source");
                    for (i, line) in text.lines().enumerate() {
                        let trimmed = line.trim();
                        for prefix in BANNED_PREFIXES {
                            if trimmed.contains(&format!("struct {prefix}"))
                                || trimmed.contains(&format!("trait {prefix}"))
                                || trimmed.contains(&format!("mod {prefix}"))
                            {
                                offences.push(format!("{file}:{}: {prefix}*", i + 1));
                            }
                        }
                    }
                }
            }
        }
    }

    assert!(
        offences.is_empty(),
        "the test suite must not define mock collaborators: {offences:#?}"
    );
}

/// Every memory bound must be a named constant, not a literal in the engine.
///
/// A bound that lives inline in `runner/engine.rs` or `runner/executor.rs` is a
/// bound nobody can find, change, or write a test against — which is how an
/// unbounded `Vec` gets reintroduced.
#[test]
fn memory_bounds_are_named_constants_not_inline_literals() {
    const BOUNDED_CALL_SITES: [(&str, &str); 4] = [
        ("src/runner/result_log.rs", "RESULT_RING_CAPACITY"),
        ("src/metrics/collector.rs", "MAX_TRACKED_ERROR_KEYS"),
        ("src/runner/executor.rs", "MAX_CAPTURED_BODY_BYTES"),
        ("src/runner/engine.rs", "max_concurrency"),
    ];

    for (file, needle) in BOUNDED_CALL_SITES {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(file);
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {file}: {e}"));
        assert!(
            text.contains(needle),
            "{file} must derive its behaviour from `{needle}` rather than an inline \
             literal, so the bound has a name, a justification, and a test"
        );
    }
}
