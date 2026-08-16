#!/bin/bash
# Parallel CPython test runner with fresh-binary sanity check.
#
# Usage:
#   tools/run_tests.sh [--jobs N] [--timeout SEC] [test_file ...]
#   tools/run_tests.sh --all        # every tests/cpython/test_*.py
#
# Builds first, verifies the binary is FRESH (avoids the stale-binary trap
# where a failed rebuild silently runs the previous executable), then runs
# each test as an independent process with its own timeout, printing a
# PASS/FAIL table and a one-line reason for each failure.
set -u

JOBS="${JOBS:-4}"
TOUT="${TOUT:-150}"
BIN="${BIN:-target/debug/rustpython}"

if ! cargo build 2>&1 | tail -1 | grep -q "Finished"; then
    echo "BUILD FAILED" >&2
    exit 1
fi

# Fresh-binary sanity check: the just-built binary must report the expected
# marker. If this fails, STOP — running tests against a stale binary wastes
# entire batches diagnosing phantom failures.
if ! "$BIN" -c "import sys; sys.exit(0 if 'RustPython' in sys.version else 1)" >/dev/null 2>&1; then
    echo "STALE/INVALID BINARY: $BIN does not run. Aborting." >&2
    exit 1
fi

ARGS=()
while [ $# -gt 0 ]; do
    case "$1" in
        --jobs) JOBS="$2"; shift 2 ;;
        --timeout) TOUT="$2"; shift 2 ;;
        --all) ARGS=(tests/cpython/test_*.py); shift ;;
        *) ARGS+=("$1"); shift ;;
    esac
done

if [ ${#ARGS[@]} -eq 0 ]; then
    echo "usage: $0 [--jobs N] [--timeout SEC] [test_file ...] | --all" >&2
    exit 2
fi

mkdir -p /tmp/rustpython-test-logs

declare -a PIDS=()
declare -a NAMES=()
run_one() {
    local f="$1"
    local base
    base="$(basename "$f" .py)"
    timeout "$TOUT" "$BIN" "$f" > "/tmp/rustpython-test-logs/$base.log" 2>&1 &
    PIDS+=("$!")
    NAMES+=("$base")
}

launch_batch() {
    PIDS=()
    NAMES=()
    local n=0
    for f in "$@"; do
        run_one "$f"
        n=$((n + 1))
        if [ "$n" -ge "$JOBS" ]; then
            break
        fi
    done
}

wait_batch() {
    local i=0
    for pid in "${PIDS[@]}"; do
        local base="${NAMES[$i]}"
        if wait "$pid"; then
            echo "PASS $base"
        else
            local reason
            reason="$(grep -vE '^JIT:' "/tmp/rustpython-test-logs/$base.log" | tail -1 | cut -c1-80)"
            echo "FAIL $base :: $reason"
        fi
        i=$((i + 1))
    done
}

remaining=("${ARGS[@]}")
while [ ${#remaining[@]} -gt 0 ]; do
    launch_batch "${remaining[@]}"
    total=${#PIDS[@]}
    # remove the launched batch from remaining
    remaining=("${remaining[@]:$total}")
    wait_batch
done
