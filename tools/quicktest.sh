#!/bin/bash
# Quick targeted testing: run only specified tests against release-lite binary
# Usage: ./tools/quicktest.sh test_io test_slice [test_dict ...]
# Or:    ./tools/quicktest.sh --all-failing  (runs all currently-failing tests in parallel)
BIN=./target/release-lite/rustpython
cd "$(dirname "$0")/.."

if [ "$1" = "--all-failing" ]; then
    shift
    TESTS=$(grep "^FAIL" /tmp/sweep_last.txt 2>/dev/null | awk '{print $2}' || echo "")
else
    TESTS="$@"
fi

if [ -z "$TESTS" ]; then
    echo "No tests specified"
    exit 1
fi

# Run up to NPROC tests in parallel
NPROC=$(nproc)
running=0
pids=()
results=()

for t in $TESTS; do
    timeout 30 $BIN "tests/cpython/$t.py" > "/tmp/qt_$t.log" 2>&1 &
    pids+=($!)
    results+=("$t")
    running=$((running+1))
    if [ $running -ge $NPROC ]; then
        wait
        running=0
    fi
done
wait

for t in "${results[@]}"; do
    status=$(tail -1 "/tmp/qt_$t.log" 2>/dev/null | grep -oE 'OK|FAILED.*|Ran [0-9]+' | head -1)
    echo "$t: $status"
done
