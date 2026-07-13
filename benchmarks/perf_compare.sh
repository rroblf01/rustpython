#!/usr/bin/env bash
# Per-benchmark timing for RustPython vs CPython
# Usage: bash benchmarks/perf_compare.sh

set -e
cd "$(dirname "$0")/.."

RUSTPYTHON="target/release/rustpython"
N=50000

echo "=========================================="
echo " RustPython vs CPython Benchmark (N=$N)"
echo "=========================================="
echo ""
echo "All times in milliseconds (lower is better)"
echo ""

# For RustPython, we use bash timing since it has no time.perf_counter
# For CPython, we use internal timing

run_rustpython() {
    local name=$1
    local code=$2
    local total=0
    for run in 1 2 3; do
        t0=$(date +%s%N 2>/dev/null || python3 -c 'import time; print(int(time.time()*1e9))')
        target/release/rustpython -c "$code" > /dev/null 2>&1
        t1=$(date +%s%N 2>/dev/null || python3 -c 'import time; print(int(time.time()*1e9))')
        local elapsed=$(( (t1 - t0) / 1000000 ))
        total=$(( total + elapsed ))
    done
    echo $(( total / 3 ))
}

run_cpython() {
    local name=$1
    local code=$2
    python3 -c "
import time
N = 50000
$code
t0 = time.perf_counter()
for _ in range(3):
    $code
t = (time.perf_counter() - t0) / 3 * 1000
print(f'{t:.2f}')
"
}

echo "+----------------------+----------+----------+-------+"
echo "| Benchmark            | CPython  | RustPy   | Ratio |"
echo "+----------------------+----------+----------+-------+"

# Arithmetic
CP=$(run_cpython arithmetic "n=0; exec('for i in range(N): n+=i; n-=i//2; n*=2; n//=3; n%=1000', {'N':N, 'n':n})")
# Convert the inline exec to work properly
CP=$(python3 -c "
import time
N=50000
def b():
    n=0
    for i in range(N):
        n+=i; n-=i//2; n*=2; n//=3; n%=1000
    return n
b()
t0=time.perf_counter()
for _ in range(3): b()
t=(time.perf_counter()-t0)/3*1000
print(f'{t:.2f}')
")
RP=$(bash -c '
t0=$(date +%s%N)
for _ in 1 2 3; do
  target/release/rustpython -c "
N=50000
n=0
for i in range(N):
    n += i
    n -= i // 2
    n *= 2
    n //= 3
    n %= 1000
print(n)
" > /dev/null 2>&1
done
t1=$(date +%s%N)
echo $(( (t1 - t0) / 3000000 ))
')
RATIO=$(python3 -c "print(f'{$RP/$CP:.2f}')" 2>/dev/null || echo "?")
printf "| %-20s | %8s | %8s | %5s |\n" "arithmetic" "${CP}ms" "${RP}ms" "${RATIO}x"

echo "+----------------------+----------+----------+-------+"
