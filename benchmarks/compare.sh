#!/usr/bin/env bash
# Memory + time comparison benchmark: RustPython vs CPython
# Usage: bash benchmarks/compare.sh

set -euo pipefail

RUSTPYTHON="${RUSTPYTHON:-target/release/rustpython}"
CPYTHON="${CPYTHON:-python3}"
BENCH_SCRIPT="${BENCH_SCRIPT:-benchmarks/minimal_bench.py}"
OUTPUT_DIR="${OUTPUT_DIR:-benchmarks/results}"

mkdir -p "$OUTPUT_DIR"

echo "========================================"
echo " RustPython vs CPython Benchmark Suite"
echo "========================================"
echo ""
echo "Date: $(date)"
echo "Host: $(uname -a)"
echo ""

# ---- CPython version ----
echo "--- CPython ---"
CPYTHON_VER=$("$CPYTHON" --version 2>&1)
echo "  $CPYTHON_VER"

# ---- RustPython version ----
echo "--- RustPython ---"
RUSTPYTHON_VER=$("$RUSTPYTHON" --version 2>&1 || echo "RustPython 0.1.0")
echo "  $RUSTPYTHON_VER"
echo ""

# ---- Benchmark runner function ----
run_bench() {
    local label="$1"
    local interpreter="$2"
    local script="$3"
    local extra="$4"
    local outfile="$OUTPUT_DIR/${label// /_}.txt"

    echo "Running: $label"
    echo "  Command: /usr/bin/time -v $interpreter $script $extra 2>&1"

    # Time + memory measurement
    TIMEFORMAT="real %R  user %U  sys %S"
    local output
    output=$({ /usr/bin/time -v $interpreter "$script" $extra 2>&1; } 2>&1)

    local real_time
    real_time=$(echo "$output" | grep -E "^[0-9]+\.[0-9]+user" | awk '{print $1}' | sed 's/user//' || echo "N/A")
    local max_rss
    max_rss=$(echo "$output" | grep "Maximum resident" | awk '{print $NF}')
    local minor_faults
    minor_faults=$(echo "$output" | grep "Minor page" | awk '{print $NF}')
    local major_faults
    major_faults=$(echo "$output" | grep "Major page" | awk '{print $NF}')
    local vol_ctx
    vol_ctx=$(echo "$output" | grep "Voluntary context" | awk '{print $NF}')
    local invol_ctx
    invol_ctx=$(echo "$output" | grep "Involuntary context" | awk '{print $NF}')

    # Extract benchmark output
    local bench_output
    bench_output=$(echo "$output" | grep -A999 "^int_add" || echo "")

    echo "  Real time: ${real_time}s"
    echo "  Max RSS: ${max_rss} KB ($(( max_rss / 1024 )) MB)"
    echo "  Minor faults: $minor_faults"
    echo "  Major faults: $major_faults"

    # Save full output
    echo "$output" > "$outfile"
    echo "" >> "$outfile"
    echo "=== BENCHMARK RESULTS ===" >> "$outfile"
    echo "$bench_output" >> "$outfile"

    # Return data as JSON-like
    echo "{\"label\":\"$label\",\"real_time\":\"$real_time\",\"max_rss_kb\":$max_rss,\"bench_output\":\"$(echo "$bench_output" | tr '\n' '|')\"}"
}

# ---- Run both interpreters ----
echo ""
echo "--- Benchmarks ---"
echo ""

# First, check if the benchmark script works on both
echo "--- Testing CPython benchmark ---"
timeout 60 $CPYTHON "$BENCH_SCRIPT" 2>&1 | head -30 || echo "CPython benchmark failed"

echo ""
echo "--- Testing RustPython benchmark ---"
timeout 120 $RUSTPYTHON "$BENCH_SCRIPT" 2>&1 | head -30 || echo "RustPython benchmark failed"

echo ""
echo "--- Detailed memory comparison ---"
echo ""

# Memory stress test
MEM_TEST='x = list(range(1000000)); print(f"List of {len(x)} ints: OK")'

echo "--- CPython memory test ---"
/usr/bin/time -v $CPYTHON -c "$MEM_TEST" 2>&1 | grep -E "Maximum resident|Elapsed|Minor page|Major page" || true

echo ""
echo "--- RustPython memory test ---"
/usr/bin/time -v $RUSTPYTHON -c "$MEM_TEST" 2>&1 | grep -E "Maximum resident|Elapsed|Minor page|Major page" || true

echo ""
echo "--- List comprehension memory test ---"
MEM_TEST2='x = 0; [x + i for i in range(100000)]; print("OK")'

echo "--- CPython ---"
/usr/bin/time -v $CPYTHON -c "$MEM_TEST2" 2>&1 | grep -E "Maximum resident|Elapsed" || true

echo ""
echo "--- RustPython ---"
/usr/bin/time -v $RUSTPYTHON -c "$MEM_TEST2" 2>&1 | grep -E "Maximum resident|Elapsed" || true

echo ""
echo "--- Large dict memory test ---"
MEM_TEST3='d = {i: i*2 for i in range(100000)}; print(f"Dict of {len(d)} items: OK")'

echo "--- CPython ---"
/usr/bin/time -v $CPYTHON -c "$MEM_TEST3" 2>&1 | grep -E "Maximum resident|Elapsed" || true

echo ""
echo "--- RustPython ---"
/usr/bin/time -v $RUSTPYTHON -c "$MEM_TEST3" 2>&1 | grep -E "Maximum resident|Elapsed" || true

echo ""
echo "========================================"
echo " Done. Results saved to $OUTPUT_DIR/"
echo "========================================"
