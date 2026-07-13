#!/usr/bin/env python3
"""Memory + performance comparison: RustPython vs CPython.
Runs benchmarks using subprocess and resource.getrusage() for RSS.

Usage:
    python3 benchmarks/compare_simple.py
"""

import subprocess
import sys
import os
import time
import json
import resource

THIS_DIR = os.path.dirname(os.path.abspath(__file__))
PROJECT_DIR = os.path.dirname(THIS_DIR)
RUSTPYTHON = os.path.join(PROJECT_DIR, "target", "release", "rustpython")
CPYTHON = "python3"

def run_with_measurement(interp: str, code: str, timeout: int = 60) -> dict:
    """Run code under interpreter, measure time and max RSS."""
    # We measure using a wrapper so we get child process RSS
    wrapper = '''
import subprocess, sys, time, resource
cmd = sys.argv[1:]
t0 = time.monotonic()
r = subprocess.run(cmd, capture_output=True, text=True, timeout=%d)
t1 = time.monotonic()
usage = resource.getrusage(resource.RUSAGE_CHILDREN)
# Return JSON
import json
print(json.dumps({
    "time": t1 - t0,
    "rss_kb": usage.ru_maxrss,
    "retcode": r.returncode,
    "stdout": r.stdout[:300],
    "stderr": r.stderr[:300]
}))
''' % timeout

    result = subprocess.run(
        [sys.executable, "-c", wrapper, interp, "-c", code],
        capture_output=True, text=True, timeout=timeout + 30
    )
    for line in result.stdout.strip().split('\n'):
        line = line.strip()
        if line.startswith('{'):
            return json.loads(line)
    return {"time": 999, "rss_kb": 0, "retcode": -1, "stdout": "", "stderr": result.stderr[:200]}


def run_benchmark_script(interp: str, script_path: str, timeout: int = 120) -> dict:
    """Run a full benchmark script and measure time/RSS."""
    wrapper = '''
import subprocess, sys, time, resource
cmd = sys.argv[1:]
t0 = time.monotonic()
r = subprocess.run(cmd, capture_output=True, text=True, timeout=%d)
t1 = time.monotonic()
usage = resource.getrusage(resource.RUSAGE_CHILDREN)
import json
print(json.dumps({
    "time": t1 - t0,
    "rss_kb": usage.ru_maxrss,
    "retcode": r.returncode,
    "stdout": r.stdout[:1000],
    "stderr": r.stderr[:300]
}))
''' % timeout

    result = subprocess.run(
        [sys.executable, "-c", wrapper, interp, script_path],
        capture_output=True, text=True, timeout=timeout + 30
    )
    for line in result.stdout.strip().split('\n'):
        line = line.strip()
        if line.startswith('{'):
            return json.loads(line)
    return {"time": 999, "rss_kb": 0, "retcode": -1, "stdout": "", "stderr": result.stderr[:200]}


def measure_benchmark(interp: str, code: str, label: str, timeout: int = 60):
    """Run a benchmark once for warmup, then 3 timed runs."""
    # Warmup
    try:
        subprocess.run([interp, "-c", code], capture_output=True, timeout=timeout)
    except:
        pass

    times = []
    rss_kb = 0
    for _ in range(3):
        result = run_with_measurement(interp, code, timeout)
        times.append(result.get("time", 0))
        rss_kb = max(rss_kb, result.get("rss_kb", 0))
    avg_time = sum(times) / len(times) if times else 0
    return avg_time, rss_kb


def main():
    if not os.path.exists(RUSTPYTHON):
        print(f"ERROR: RustPython not found at {RUSTPYTHON}")
        print("Run: cd {} && cargo build --release".format(PROJECT_DIR))
        sys.exit(1)

    print("=" * 70)
    print("RustPython vs CPython - Performance & Memory Comparison")
    print("=" * 70)
    print()
    print(f"CPython:     {subprocess.run([CPYTHON, '--version'], capture_output=True, text=True).stdout.strip()}")
    rp_ver = subprocess.run([RUSTPYTHON, '--version'], capture_output=True, text=True)
    print(f"RustPython:  {rp_ver.stdout.strip() or '0.1.0'}")
    print(f"Date:        {time.strftime('%Y-%m-%d %H:%M:%S')}")
    print(f"Host:        {os.uname().nodename}")
    print()

    # ── Benchmarks ──
    benchmarks = [
        ("int_add_1M", "x = 0\nfor i in range(1000000):\n    x += i\nprint(x)"),
        ("list_build_500K", "lst = [i for i in range(500000)]\nprint(len(lst))"),
        ("dict_build_200K", "d = {i: i*2 for i in range(200000)}\nprint(len(d))"),
        ("str_join_50K", "s = ''.join(str(i) for i in range(50000))\nprint(len(s))"),
        ("loop_2M", "s = 0\nfor i in range(2000000):\n    s += i % 10\nprint(s)"),
        ("fib_25", """
def fib(n):
    if n < 2:
        return n
    return fib(n-1) + fib(n-2)
print(fib(25))
"""),
        ("list_append_100K", "lst = []\nfor i in range(100000):\n    lst.append(i)\nprint(len(lst))"),
        ("list_sum_100K", "lst = list(range(100000))\nprint(sum(lst))"),
    ]

    # Memory stress tests (need bigger types)
    mem_benchmarks = [
        ("mem_large_list", "x = list(range(1000000)); print(len(x))"),
        ("mem_large_dict", "d = {i: i*2 for i in range(200000)}; print(len(d))"),
        ("mem_large_str", "s = 'x' * 10000000; print(len(s))"),
    ]

    all_results = {}

    for i, (bname, code) in enumerate(benchmarks):
        print(f"Benchmark {i+1}/{len(benchmarks)}: {bname} ...")
        ct, crss = measure_benchmark(CPYTHON, code, bname)
        rt, rrss = measure_benchmark(RUSTPYTHON, code, bname, timeout=120)
        all_results[bname] = {"cpython": (ct, crss), "rustpython": (rt, rrss)}
        ratio = rt / ct if ct > 0 else 0
        print(f"  CPython:     {ct*1000:8.2f} ms,  RSS={crss/1024:6.1f} MB")
        print(f"  RustPython:  {rt*1000:8.2f} ms,  RSS={rrss/1024:6.1f} MB")
        print(f"  Speed:       {ratio:.2f}x slower than CPython")
        if rt > 0 and ct > 0:
            rss_ratio = rrss / crss if crss > 0 else 0
            print(f"  Memory:      {rss_ratio:.2f}x CPython RSS ({'uses more' if rss_ratio > 1.0 else 'uses less'})")
        print()

    # Memory stress
    print("── Memory Stress Tests ──")
    for bname, code in mem_benchmarks:
        rt, rrss = measure_benchmark(RUSTPYTHON, code, bname, timeout=120)
        ct, crss = measure_benchmark(CPYTHON, code, bname)
        all_results[bname] = {"cpython": (ct, crss), "rustpython": (rt, rrss)}
        ratio = rt / ct if ct > 0 else 0
        rss_ratio = rrss / crss if crss > 0 else 0
        print(f"  {bname:20s}:  RustPython time={rt*1000:8.2f}ms (x{ratio:.2f}), RSS={rrss/1024:5.1f}MB (x{rss_ratio:.2f} CPython)")

    # ── Full benchmark suite ──
    print()
    print("── Running Full Benchmark Suite ──")
    bench_script = os.path.join(THIS_DIR, "minimal_bench.py")

    print("  CPython...")
    c_result = run_benchmark_script(CPYTHON, bench_script, timeout=120)
    ct = c_result.get("time", 0)
    crss = c_result.get("rss_kb", 0)
    print(f"    Time: {ct:.3f}s  RSS: {crss} KB ({crss/1024:.1f} MB)")

    print("  RustPython...")
    r_result = run_benchmark_script(RUSTPYTHON, bench_script, timeout=180)
    rt = r_result.get("time", 0)
    rrss = r_result.get("rss_kb", 0)
    print(f"    Time: {rt:.3f}s  RSS: {rrss} KB ({rrss/1024:.1f} MB)")

    ratio = rt / ct if ct > 0 else 0
    print(f"  Speed ratio: {ratio:.2f}x slower than CPython")
    print(f"  RSS ratio:   {rrss/crss:.2f}x CPython RSS")

    # ── Summary table ──
    print()
    print("=" * 70)
    print("SUMMARY TABLE")
    print("=" * 70)
    print(f"{'Benchmark':22s} {'CPy (ms)':>10s} {'RPy (ms)':>10s} {'Ratio':>8s} {'CPy RSS':>10s} {'RPy RSS':>10s}")
    print("-" * 70)
    for bname, _ in benchmarks:
        ct, crss = all_results.get(bname, {}).get("cpython", (0, 0))
        rt, rrss = all_results.get(bname, {}).get("rustpython", (0, 0))
        ratio = rt / ct if ct > 0 else 0
        print(f"{bname:22s} {ct*1000:>8.2f}  {rt*1000:>8.2f}  {ratio:>7.2f}x {crss/1024:>7.1f}MB {rrss/1024:>7.1f}MB")
    print("-" * 70)
    ct, crss = all_results.get(list(benchmarks)[-1] if benchmarks else "", {}).get("cpython", (0, 0))
    rt, rrss = all_results.get(list(benchmarks)[-1] if benchmarks else "", {}).get("rustpython", (0, 0))
    # Summary line for full suite
    print(f"{'Full suite:':22s} {ct*1000:>8.2f}  {rt*1000:>8.2f}  {rt/ct:>7.2f}x {crss/1024:>7.1f}MB {rrss/1024:>7.1f}MB" if ct > 0 else "")
    print()

    # Save to JSON
    results_file = os.path.join(THIS_DIR, "results", "benchmark_results.json")
    os.makedirs(os.path.dirname(results_file), exist_ok=True)
    with open(results_file, "w") as f:
        json.dump(all_results, f, indent=2)
    print(f"Results saved to {results_file}")


if __name__ == "__main__":
    main()
