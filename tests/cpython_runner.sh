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

export PYTHONPATH="${VENV_SITE}${PYTHONPATH:+:$PYTHONPATH}"

start=$(date +%s%N)
timeout "$TIMEOUT" "$BINARY" "$TEST_FILE" > "$logfile" 2>&1
exit_code=$?
end=$(date +%s%N)
elapsed=$(( (end - start) / 1000000 ))

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
