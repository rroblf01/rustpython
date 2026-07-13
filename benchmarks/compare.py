#!/usr/bin/env python3
"""Memory and time comparison: RustPython vs CPython.

Usage:
    python3 benchmarks/compare.py [--rustpython PATH] [--cpython PATH]

Measures RSS (resident set size) and wall-clock time for each interpreter.
"""

import subprocess
import sys
import os
import time

RUSTPYTHON_DEFAULT = os.path.join(os.path.dirname(os.path.dirname(
    os.path.abspath(__file__))), "target", "release", "rustpython")
CPYTHON_DEFAULT = "python3"


def measure(interpreter: str, code: str) -> dict:
    """Run code under interpreter, measure max RSS and wall time."""
    # Use subprocess with time measurement
    # We use a wrapper Python script that reports max RSS via resource module
    wrapper = '''
import subprocess, sys, os, time, resource

def measure():
    cmd = sys.argv[1:]
    if not cmd:
        return
    # Warmup
    for _ in range(2):
        r = subprocess.run(cmd, capture_output=True, text=True, timeout=30)
    
    # Timed runs
    times = []
    rss = 0
    for _ in range(3):
        t0 = time.monotonic()
        r = subprocess.run(cmd, capture_output=True, text=True, timeout=30)
        t1 = time.monotonic()
        times.append(t1 - t0)
        # Get max RSS of child using resource
        usage = resource.getrusage(resource.RUSAGE_CHILDREN)
        rss = max(rss, usage.ru_maxrss)
    
    avg_time = sum(times) / len(times)
    # Print JSON result
    print(f'{{"time_sec": {avg_time:.6f}, "max_rss_kb": {rss}, "stdout": {repr(r.stdout)}, "stderr": {repr(r.stderr[:200])}}}')

if __name__ == "__main__":
    measure()
'''
    cmd = [interpreter, "-c", code]
    result = subprocess.run(
        [sys.executable, "-c", wrapper] + cmd,
        capture_output=True, text=True, timeout=60
    )
    # Parse JSON from stdout
    for line in result.stdout.strip().split('\n'):
        line = line.strip()
        if line.startswith('{'):
            import json
            return json.loads(line)
    return {"error": f"Could not parse: {result.stdout[:200]}", "time_sec": 0, "max_rss_kb": 0}


def measure_script(interpreter: str, script_path: str) -> dict:
    """Run a Python script and measure performance."""
    wrapper = '''
import subprocess, sys, time, resource, json

def measure():
    cmd = sys.argv[1:]
    if not cmd:
        return
    # Warmup run
    subprocess.run(cmd, capture_output=True, text=True, timeout=120)
    
    # Timed runs
    times = []
    rss = 0
    for _ in range(3):
        t0 = time.monotonic()
        r = subprocess.run(cmd, capture_output=True, text=True, timeout=120)
        t1 = time.monotonic()
        times.append(t1 - t0)
        usage = resource.getrusage(resource.RUSAGE_CHILDREN)
        rss = max(rss, usage.ru_maxrss)
    
    avg_time = sum(times) / len(times)
    output = r.stdout[:500] if r.returncode == 0 else r.stderr[:500]
    print(f'{{"time_sec": {avg_time:.6f}, "max_rss_kb": {rss}, "exit_code": {r.returncode}, "output": {json.dumps(output)}}}')

if __name__ == "__main__":
    measure()
'''
    cmd = [interpreter, script_path]
    result = subprocess.run(
        [sys.executable, "-c", wrapper] + cmd,
        capture_output=True, text=True, timeout=180
    )
    for line in result.stdout.strip().split('\n'):
        line = line.strip()
        if line.startswith('{'):
            import json
            return json.loads(line)
    return {"error": f"Could not parse: {result.stdout[:200]}", "time_sec": 0, "max_rss_kb": 0}


def main():
    rustpython = os.environ.get("RUSTPYTHON", RUSTPYTHON_DEFAULT)
    cpython = os.environ.get("CPYTHON", CPYTHON_DEFAULT)
    script = os.environ.get("BENCH_SCRIPT", "benchmarks/minimal_bench.py")

    if not os.path.exists(rustpython):
        print(f"RustPython binary not found at {rustpython}")
        print(f"Build it with: cargo build --release")
        sys.exit(1)

    print("=" * 60)
    print("RustPython vs CPython — Performance & Memory Comparison")
    print("=" * 60)
    print(f"Date: {time.strftime('%Y-%m-%d %H:%M:%S')}")
    print()

    # Version info
    print(f"CPython:     {subprocess.run([cpython, '--version'], capture_output=True, text=True).stdout.strip()}")
    rp_ver = subprocess.run([rustpython, '--version'], capture_output=True, text=True)
    print(f"RustPython:  {rp_ver.stdout.strip() or rp_ver.stderr.strip() or '0.1.0'}")
    print()

    # ── Individual benchmarks ──

    benchmarks = [
        # (name, code)
        ("int_add", "x = 0\nfor i in range(1000000):\n    x += i\nprint(x)"),
        ("list_build", "lst = [i for i in range(500000)]\nprint(len(lst))"),
        ("dict_build", "d = {i: i*2 for i in range(200000)}\nprint(len(d))"),
        ("str_join", "s = ''.join(str(i) for i in range(50000))\nprint(len(s))"),
        ("loop", "s = 0\nfor i in range(2000000):\n    s += i % 10\nprint(s)"),
        ("fib_recursive", """
def fib(n):
    if n < 2:
        return n
    return fib(n-1) + fib(n-2)
print(fib(25))
"""),
    ]

    print("── Individual Benchmarks ──")
    print(f"{'Benchmark':20s} {'Interp':12s} {'Time (s)':>10s}  {'RSS (KB)':>10s}  {'RSS (MB)':>10s}")
    print("-" * 64)

    all_results = {}
    for name in ['cpython', 'rustpython']:
        interp = cpython if name == 'cpython' else rustpython
        all_results[name] = {}
        for bname, code in benchmarks:
            result = measure(interp, code)
            t = result.get('time_sec', 0)
            rss = result.get('max_rss_kb', 0)
            all_results[name][bname] = result
            label = name[:12]
            print(f"{bname:20s} {label:12s} {t:10.4f}  {rss:>10d}  {rss/1024:>10.2f}")

    # Speed ratio
    print()
    print("── Speed Comparison (CPython = 1.0x) ──")
    for bname, _ in benchmarks:
        ct = all_results['cpython'][bname].get('time_sec', 0)
        rt = all_results['rustpython'][bname].get('time_sec', 0)
        if ct > 0 and rt > 0:
            ratio = rt / ct
            print(f"{bname:20s}  RustPython is {ratio:.2f}x slower than CPython")
        elif ct == 0:
            print(f"{bname:20s}  CPython: no data")
        else:
            print(f"{bname:20s}  RustPython: no data")

    # ── Full benchmark suite ──
    print()
    print("── Full Benchmark Suite ──")
    bench_path = os.path.join(os.path.dirname(os.path.dirname(
        os.path.abspath(__file__))), script)

    for name, interp in [("CPython", cpython), ("RustPython", rustpython)]:
        print(f"\n  {name}:")
        result = measure_script(interp, bench_path)
        t = result.get('time_sec', 0)
        rss = result.get('max_rss_kb', 0)
        print(f"    Time: {t:.4f} s")
        print(f"    RSS:  {rss} KB ({rss/1024:.1f} MB)")
        for line in result.get('output', '').split('\\n'):
            if line.strip():
                print(f"    {line}")

    # ── Memory stress test ──
    print()
    print("── Memory Stress Test ──")
    mem_tests = [
        ("large_list", "x = list(range(1000000)); print(f'List: {len(x)}')"),
        ("large_dict", "d = {i: i*2 for i in range(200000)}; print(f'Dict: {len(d)}')"),
        ("large_str", "s = 'x' * 10000000; print(f'Str: {len(s)}')"),
    ]

    print(f"{'Test':20s} {'Interp':12s} {'Time (s)':>10s}  {'RSS (KB)':>10s}  {'RSS (MB)':>10s}")
    print("-" * 64)
    for name in ['cpython', 'rustpython']:
        interp = cpython if name == 'cpython' else rustpython
        for tname, code in mem_tests:
            result = measure(interp, code)
            t = result.get('time_sec', 0)
            rss = result.get('max_rss_kb', 0)
            label = name[:12]
            print(f"{tname:20s} {label:12s} {t:10.4f}  {rss:>10d}  {rss/1024:>10.2f}")

    # ── Summary ──
    print()
    print("=" * 60)
    print("SUMMARY")
    print("=" * 60)
    for bname, _ in benchmarks:
        ct = all_results['cpython'][bname].get('time_sec', 0)
        rt = all_results['rustpython'][bname].get('time_sec', 0)
        crss = all_results['cpython'][bname].get('max_rss_kb', 0)
        rrss = all_results['rustpython'][bname].get('max_rss_kb', 0)
        if ct > 0 and rt > 0:
            print(f"{bname:20s}  time: {rt/ct:.2f}x CPython  |  RSS: RustPython={rrss/1024:.1f}MB CPython={crss/1024:.1f}MB")
    print()


if __name__ == "__main__":
    main()
