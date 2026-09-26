#!/usr/bin/env bash
# =============================================================================
# safe-cargo.sh — memory-capped wrapper around every cargo invocation
# =============================================================================
# WHY THIS EXISTS
#   This machine has 15 GB RAM and 8 cores. A release build of this crate's
#   196-dependency tree previously ran with `jobs = 8` plus thin LTO and
#   `codegen-units = 1`. Parallel rustc codegen plus the LTO link spike
#   exceeded physical RAM and the OOM killer took out the desktop session —
#   twice.
#
#   Nothing in Cargo, rustc, or clippy enforces a memory ceiling. The ceiling
#   has to be imposed from outside. This script does that with a cgroup v2
#   memory cap, so an over-budget build is KILLED instead of taking the machine
#   with it. A killed build is a failed build; a killed machine is a lost
#   session.
#
# USAGE
#   scripts/safe-cargo.sh check
#   scripts/safe-cargo.sh clippy
#   scripts/safe-cargo.sh test --all-targets
#   scripts/safe-cargo.sh build --release
#   SAFE_CARGO_MEM=8G scripts/safe-cargo.sh build --release
#
# EXIT CODES
#   0   cargo succeeded
#   1   cargo failed for an ordinary reason
#   137  killed by the memory cap (OOM) — raise SAFE_CARGO_MEM or lower SAFE_CARGO_JOBS
# =============================================================================
set -euo pipefail

# Total memory the build tree (cargo + all rustc children) may consume.
# Deliberately well under physical RAM so the desktop, compositor, and browser
# keep their headroom.
SAFE_CARGO_MEM="${SAFE_CARGO_MEM:-6G}"

# Parallel codegen units. Each one can hold hundreds of MB to a few GB during
# optimisation and LTO, so this is the primary memory multiplier.
SAFE_CARGO_JOBS="${SAFE_CARGO_JOBS:-2}"

# Keep debug info out of the picture unless explicitly building a debug profile;
# it is a large, low-value memory cost for verification builds.
export CARGO_BUILD_JOBS="$SAFE_CARGO_JOBS"
export CARGO_TERM_PROGRESS_WHEN=never

if [[ $# -eq 0 ]]; then
    echo "usage: scripts/safe-cargo.sh <cargo-subcommand> [args...]" >&2
    exit 1
fi

# systemd-run gives us a real cgroup memory cap. If it is unavailable, fall back
# to running uncapped but with the reduced job count, and say so loudly.
run_capped() {
    systemd-run \
        --quiet \
        --scope \
        --user \
        --property="MemoryMax=${SAFE_CARGO_MEM}" \
        --property="MemorySwapMax=0" \
        -- cargo "$@"
}

run_uncapped_note() {
    echo "WARNING: systemd-run unavailable; running without a hard memory cap." >&2
    echo "         Only ${SAFE_CARGO_JOBS} job(s) will run. Do not raise this on a 15 GB host." >&2
    cargo "$@"
}

if command -v systemd-run >/dev/null 2>&1; then
    set +e
    run_capped "$@"
    status=$?
    set -e
    if [[ $status -eq 137 ]]; then
        echo >&2
        echo "BUILD KILLED by the ${SAFE_CARGO_MEM} memory cap." >&2
        echo "This is the failsafe working. To proceed legitimately:" >&2
        echo "  - lower SAFE_CARGO_JOBS (currently ${SAFE_CARGO_JOBS})" >&2
        echo "  - or raise SAFE_CARGO_MEM (currently ${SAFE_CARGO_MEM}) if headroom exists" >&2
        echo "  - or build a narrower target: cargo check instead of build --release" >&2
    fi
    exit $status
else
    set +e
    run_uncapped_note "$@"
    status=$?
    set -e
    exit $status
fi
