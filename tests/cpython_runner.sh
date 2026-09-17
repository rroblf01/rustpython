#!/bin/bash
# Run a single CPython test file and classify the result.
# Usage: cpython_runner.sh <binary> <test_file> <log_dir> <timeout_secs> <venv_site>
BINARY="$1"
TEST_FILE="$2"
LOG_DIR="$3"
TIMEOUT="$4"
VENV_SITE="$5"

name=$(basename "$TEST_FILE" .py)
logfile="$LOG_DIR/$name.log"

# Real CPython test files (os_helper.TESTFN et al.) create scratch files/dirs
# using RELATIVE paths, on the assumption that the real regrtest harness has
# already chdir'd into a scratch directory before running them. This runner
# didn't, so every such file (e.g. dbm/shelve tests' `@test_<pid>_tmp...`
# files) landed directly in the project's own working directory instead —
# permanent repo clutter, growing by however many tests create scratch files
# on every single sweep. Resolve the binary/test-file paths to absolute
# BEFORE changing directory, run from a fresh per-test scratch dir, then
# remove it afterward — matches what regrtest itself does, and confines
# ALL such stray output (not just dbm's) to a throwaway location.
BINARY="$(cd "$(dirname "$BINARY")" && pwd)/$(basename "$BINARY")"
TEST_FILE="$(cd "$(dirname "$TEST_FILE")" && pwd)/$(basename "$TEST_FILE")"
LOG_DIR="$(cd "$LOG_DIR" && pwd)"
logfile="$LOG_DIR/$name.log"
scratch_dir="$(mktemp -d "/tmp/rustpython-cpython-scratch.XXXXXX")"

export PYTHONPATH="${VENV_SITE}${PYTHONPATH:+:$PYTHONPATH}"

start=$(date +%s%N)
# `--kill-after=10`: plain `timeout N` sends SIGTERM at N seconds and then
# waits INDEFINITELY for the child to actually exit — a process stuck deep
# inside a single huge allocator call (a runaway Vec/String growing without
# bound, e.g. an interpreter-level infinite loop that a missing/incomplete
# feature failed to interrupt) may not respond to SIGTERM promptly, and can
# keep growing memory well past the nominal timeout until it either
# eventually notices the signal or gets OOM-killed by the kernel first —
# confirmed real: a `sys.settrace`-dependent test (`test_sys_settrace.py`'s
# `test_no_jump_infinite_while_loop`, relying on a debugger line-jump this
# interpreter's `sys.settrace` doesn't actually support — a real, separate,
# larger gap, not fixed here) ran for several MINUTES past its 180s budget
# and got the whole system's kernel OOM-killer involved, taking other
# unrelated processes down with it. `--kill-after=10` forces an
# UNCATCHABLE SIGKILL 10s after the initial SIGTERM if the process hasn't
# exited by then, giving every single test file a hard, unconditional
# memory/runtime backstop regardless of what specific bug causes a runaway.
#
# Peak-RSS accounting: while `timeout` is alive, poll the kernel's VmHWM
# high-watermark (exact peak, no sampling loss for the maximum) over the
# `timeout` process plus two levels of descendants (the interpreter itself
# plus anything the test spawns, e.g. test_subprocess children). Reported
# as a tree sum in MB — shared pages are counted once per process, so it
# overstates true unique memory by a small constant; valid for comparing
# runs against each other, which is its purpose (RAM baseline for the
# compat hunt). Fast tests (<0.2s) may record a partial sample.
(cd "$scratch_dir" && timeout --kill-after=10 "$TIMEOUT" "$BINARY" "$TEST_FILE") > "$logfile" 2>&1 &
timeout_pid=$!
peak_kb=0
while kill -0 $timeout_pid 2>/dev/null; do
  total_kb=0
  for p in $timeout_pid $(ps -o pid= --ppid $timeout_pid 2>/dev/null); do
    for q in $p $(ps -o pid= --ppid $p 2>/dev/null); do
      hwm=$(awk '/^VmHWM:/{print $2}' /proc/$q/status 2>/dev/null)
      total_kb=$((total_kb + ${hwm:-0}))
    done
  done
  [ "$total_kb" -gt "$peak_kb" ] && peak_kb=$total_kb
  sleep 0.2
done
wait $timeout_pid
exit_code=$?
end=$(date +%s%N)
elapsed=$(( (end - start) / 1000000 ))
peak_mb=$(( (peak_kb + 1023) / 1024 ))
rm -rf "$scratch_dir"

# Classify the result
if [ $exit_code -eq 0 ]; then
    echo "PASS|$name|${elapsed}ms|${peak_mb}MB"
elif [ $exit_code -eq 124 ]; then
    echo "TIMEOUT|$name|${elapsed}ms|${peak_mb}MB"
else
    # If test raised unittest.SkipTest (platform-specific module not
    # available, e.g. winreg on Linux, _zstd not built, etc.) it
    # correctly skipped — don't count as FAIL. CPython's regrtest
    # counts these as SKIPPED, not FAILED. Same for ResourceDenied
    # (test requires a resource like a terminal, network, or audio
    # device that is unavailable in this environment, e.g. curses).
    if grep -q "SkipTest\|ResourceDenied" "$logfile"; then
        echo "SKIP|$name|${elapsed}ms|${peak_mb}MB"
        # Normalize exit code so Makefile's `set -e` / failure check
        # doesn't treat a correct skip as a sweep failure.
        exit 0
    fi
    # Check the first line of meaningful output (skip JIT warnings)
    first_error=$(grep -v "^JIT:" "$logfile" | head -1)
    # If the first meaningful line is a Parse error from the interpreter,
    # the test never actually ran — classify it as PARSE_ERROR.
    if echo "$first_error" | grep -q "^Parse error\|^SyntaxError:"; then
        echo "PARSE_ERROR|$name|${elapsed}ms|${peak_mb}MB"
    else
        echo "FAIL|$name|${elapsed}ms|${peak_mb}MB"
    fi
fi
exit $exit_code
