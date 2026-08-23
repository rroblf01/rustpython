#!/bin/bash
# Parallel CPython test runner with fresh-binary sanity check.
#
# Usage:
#   tools/run_tests.sh [--jobs N] [--timeout SEC] [--debug] [test_file ...]
#   tools/run_tests.sh --all        # every tests/cpython/test_*.py
#
# Defaults to the RELEASE binary (5-7x faster than debug — test_math goes
# 218s -> 37s), so the ~21s incremental release build is amortized over the
# whole batch. Use --debug to iterate a single small test with a 2s build.
# Builds first, verifies the binary is FRESH (avoids the stale-binary trap
# where a failed rebuild silently runs the previous executable), then runs
# each test as an independent process with its own timeout, printing a
# PASS/FAIL table and a one-line reason for each failure.
set -u

# Default to ALL cores — the sweep is embarrassingly parallel (one process
# per test) and was leaving 2/3 of the machine idle at the old JOBS=4.
JOBS="${JOBS:-$(nproc)}"
TOUT="${TOUT:-60}"   # 150s made legitimately-slow tests (getrandbits_2G etc.) stall the sweep
MODE="${MODE:-release}"
SHOW_TIME="${SHOW_TIME:-0}"

ARGS=()
while [ $# -gt 0 ]; do
    case "$1" in
        --jobs) JOBS="$2"; shift 2 ;;
        --timeout) TOUT="$2"; shift 2 ;;
        --debug) MODE="debug"; shift ;;
        --time) SHOW_TIME=1; shift ;;
        --skip)
            # --skip name1,name2,... exclude files by basename.
            # NOTE: applied when --all is expanded below (order-independent).
            IFS=',' read -ra SKIP_LIST <<< "$2"; shift 2 ;;
        --all) ALL_MODE=1; shift ;;
        *) ARGS+=("$1"); shift ;;
    esac
done

# Expand --all honoring any --skip list regardless of argument order.
if [ "${ALL_MODE:-0}" = 1 ]; then
    for f in tests/cpython/test_*.py; do
        base="$(basename "$f" .py)"
        skip_it=0
        for sk in ${SKIP_LIST[@]:-}; do
            [ "$base" = "$sk" ] && skip_it=1 && break
        done
        [ $skip_it = 0 ] && ARGS+=("$f")
    done
fi

if [ "$MODE" = "release" ]; then
    BIN="${BIN:-target/release/rustpython}"
    if ! cargo build --release 2>&1 | tail -1 | grep -q "Finished"; then
        echo "BUILD FAILED" >&2
        exit 1
    fi
else
    BIN="${BIN:-target/debug/rustpython}"
    if ! cargo build 2>&1 | tail -1 | grep -q "Finished"; then
        echo "BUILD FAILED" >&2
        exit 1
    fi
fi

# Fresh-binary sanity check: the just-built binary must report the expected
# marker. If this fails, STOP — running tests against a stale binary wastes
# entire batches diagnosing phantom failures.
if ! "$BIN" -c "import sys; sys.exit(0 if 'RustPython' in sys.version else 1)" >/dev/null 2>&1; then
    echo "STALE/INVALID BINARY: $BIN does not run. Aborting." >&2
    exit 1
fi

if [ ${#ARGS[@]} -eq 0 ]; then
    echo "usage: $0 [--jobs N] [--timeout SEC] [--debug] [test_file ...] | --all" >&2
    exit 2
fi

mkdir -p /tmp/rustpython-test-logs

declare -a PIDS=()
declare -a NAMES=()
declare -a T0MS=()

# Launch ONE test as a detached job. The wrapper subshell records the true
# end time and exit code INTO THE LOG, so the reaper can detect completion
# by polling the file — no barrier waits, no process-state races.
launch_one() {
    local f="$1"
    local base
    base="$(basename "$f" .py)"
    local log="/tmp/rustpython-test-logs/$base.log"
    T0MS+=("$(date +%s%3N)")
    (
        timeout -k 5 "$TOUT" "$BIN" "$f" > "$log" 2>&1
        echo "RC=$?" >> "$log"
        echo "END_EPOCH $(date +%s%3N)" >> "$log"
    ) &
    PIDS+=("$!")
    NAMES+=("$base")
}

# Reap every in-flight job whose log already carries its END_EPOCH marker,
# printing PASS/FAIL (+ true duration with --time) and compacting the
# in-flight arrays. Completion-order output, not launch-order.
reap_finished() {
    local keep_pids=() keep_names=() keep_t0=()
    local i
    for i in "${!PIDS[@]}"; do
        local pid="${PIDS[$i]}" base="${NAMES[$i]}"
        local log="/tmp/rustpython-test-logs/$base.log"
        if [ -f "$log" ] && grep -q "^END_EPOCH " "$log" 2>/dev/null; then
            wait "$pid" 2>/dev/null
            local rc
            rc="$(grep -oE '^RC=-?[0-9]+' "$log" 2>/dev/null | head -1 | cut -d= -f2)"
            local suffix=""
            if [ "$SHOW_TIME" = 1 ]; then
                local end
                end="$(grep -oE 'END_EPOCH [0-9]+' "$log" 2>/dev/null | awk '{print $2}')"
                if [ -n "$end" ]; then
                    local el=$(( end - ${T0MS[$i]} ))
                    suffix=" [$((el / 1000)).$(( (el % 1000) / 100 ))s]"
                fi
            fi
            if [ "$rc" = "0" ]; then
                echo "PASS $base$suffix"
            else
                local reason
                reason="$(grep -vE "^JIT:" "$log" 2>/dev/null | grep -v "^RC=" | grep -v "^END_EPOCH" | tail -1 | cut -c1-80)"
                echo "FAIL $base :: $reason$suffix"
            fi
        else
            keep_pids+=("$pid")
            keep_names+=("$base")
            keep_t0+=("${T0MS[$i]}")
        fi
    done
    PIDS=("${keep_pids[@]}")
    NAMES=("${keep_names[@]}")
    T0MS=("${keep_t0[@]}")
}

# Job-server main loop: keep exactly min(JOBS, remaining) tests in flight,
# refilling the instant any slot frees up (no batch barriers — one hung or
# core-dumping test no longer stalls eleven healthy slots).
SWEEP_T0=${SECONDS}
next=0
total=${#ARGS[@]}
while [ "$next" -lt "$total" ] && [ "${#PIDS[@]}" -lt "$JOBS" ]; do
    launch_one "${ARGS[$next]}"
    next=$((next + 1))
done
while [ "${#PIDS[@]}" -gt 0 ]; do
    sleep 0.1
    reap_finished
    while [ "$next" -lt "$total" ] && [ "${#PIDS[@]}" -lt "$JOBS" ]; do
        launch_one "${ARGS[$next]}"
        next=$((next + 1))
    done
done
if [ "$SHOW_TIME" = 1 ] || [ "$total" -gt 5 ]; then
    echo "TOTAL_WALL $(( SECONDS - SWEEP_T0 ))s jobs=$JOBS tests=$total"
fi
