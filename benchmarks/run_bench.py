#!/usr/bin/env python3
"""
Benchmark comparison runner.
Compares RustPython JIT vs CPython for the JIT benchmark suite.

Usage:
  python3 benchmarks/run_bench.py               # runs both
  python3 benchmarks/run_bench.py --rust-only   # only RustPython
"""

import subprocess
import sys
import time
import os

BENCH_SCRIPT = "benchmarks/jit_bench.py"
RUSTPYTHON_BIN = "cargo run --release --"

def run_rustpython():
    """Run benchmark under RustPython."""
    print("=== RustPython (JIT) ===")
    t0 = time.perf_counter()
    result = subprocess.run(
        RUSTPYTHON_BIN.split() + [BENCH_SCRIPT],
        capture_output=True, text=True, timeout=120,
        cwd=os.path.dirname(os.path.abspath(__file__)) + "/.."
    )
    t = time.perf_counter() - t0
    print(result.stdout)
    if result.stderr:
        print("STDERR:", result.stderr[:500])
    print(f"Total wall time: {t:.2f}s")
    print()
    return t, result.stdout

def run_cpython():
    """Run benchmark under system CPython."""
    print("=== CPython ===")
    t0 = time.perf_counter()
    result = subprocess.run(
        [sys.executable, BENCH_SCRIPT],
        capture_output=True, text=True, timeout=120,
        cwd=os.path.dirname(os.path.abspath(__file__)) + "/.."
    )
    t = time.perf_counter() - t0
    print(result.stdout)
    if result.stderr:
        print("STDERR:", result.stderr[:500])
    print(f"Total wall time: {t:.2f}s")
    print()
    return t, result.stdout

def parse_times(output):
    """Parse benchmark output lines to extract times."""
    times = {}
    for line in output.split('\n'):
        parts = line.strip().split()
        if len(parts) >= 4 and parts[-2].endswith('s'):
            try:
                name = parts[1]
                t = float(parts[-2].rstrip('s'))
                times[name] = t
            except (ValueError, IndexError):
                pass
    return times

def main():
    print("=" * 70)
    print("JIT BENCHMARK COMPARISON")
    print("=" * 70)
    print()

    rust_time, rust_out = run_rustpython()
    cpy_time, cpy_out = run_cpython()

    rust_times = parse_times(rust_out)
    cpy_times = parse_times(cpy_out)

    print("=" * 70)
    print("SUMMARY: RustPython vs CPython")
    print("=" * 70)
    print(f"{'Benchmark':<25s} {'RustPython':>12s} {'CPython':>12s} {'Ratio':>10s}")
    print("-" * 60)

    all_names = sorted(set(list(rust_times.keys()) + list(cpy_times.keys())))
    for name in all_names:
        r = rust_times.get(name, 0)
        c = cpy_times.get(name, 0)
        if r > 0 and c > 0:
            ratio = r / c
            print(f"{name:<25s} {r:>8.4f}s  {c:>8.4f}s  {ratio:>7.2f}x")
        elif r > 0:
            print(f"{name:<25s} {r:>8.4f}s  {'N/A':>12s}")
        else:
            print(f"{name:<25s} {'N/A':>12s} {c:>8.4f}s")

    print("-" * 60)
    if cpy_time > 0:
        print(f"{'OVERALL':<25s} {rust_time:>8.2f}s  {cpy_time:>8.2f}s  {rust_time/cpy_time:>7.2f}x")
    print()

if __name__ == "__main__":
    main()
