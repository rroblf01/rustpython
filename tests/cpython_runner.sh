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
(cd "$scratch_dir" && timeout "$TIMEOUT" "$BINARY" "$TEST_FILE") > "$logfile" 2>&1
exit_code=$?
end=$(date +%s%N)
elapsed=$(( (end - start) / 1000000 ))
rm -rf "$scratch_dir"

# Classify the result
if [ $exit_code -eq 0 ]; then
    echo "PASS|$name|${elapsed}ms"
elif [ $exit_code -eq 124 ]; then
    echo "TIMEOUT|$name|${elapsed}ms"
else
    # Check the first line of meaningful output (skip JIT warnings)
    first_error=$(grep -v "^JIT:" "$logfile" | head -1)
    # If the first meaningful line is a Parse error from the interpreter,
    # the test never actually ran — classify it as PARSE_ERROR.
    if echo "$first_error" | grep -q "^Parse error\|^SyntaxError:"; then
        echo "PARSE_ERROR|$name|${elapsed}ms"
    else
        echo "FAIL|$name|${elapsed}ms"
    fi
fi
exit $exit_code
