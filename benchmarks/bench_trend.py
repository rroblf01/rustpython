#!/usr/bin/env python3
"""Trend harness: RustPython vs CPython — time + RSS + CPython-suite aggregate.

Measures wall time + peak RSS for the quickbench and membench suites under
both interpreters, and (with --tests) the aggregate CPython test-suite
pass/fail counts. Appends one row to benchmarks/results/trend.json keyed by
git commit, and prints a comparison table.

Usage:
    python3 benchmarks/bench_trend.py            # time + RSS only
    python3 benchmarks/bench_trend.py --tests    # + CPython suite aggregate
    python3 benchmarks/bench_trend.py --quick    # smaller N, faster
"""

import subprocess
import sys
import os
import time
import json
import resource

THIS_DIR = os.path.dirname(os.path.abspath(__file__))
PROJECT = os.path.dirname(THIS_DIR)
RELEASE = os.path.join(PROJECT, "target", "release", "rustpython")
CPYTHON = "python3"
RESULTS_DIR = os.path.join(THIS_DIR, "results")
TREND_FILE = os.path.join(RESULTS_DIR, "trend.json")
QUICKBENCH = os.path.join(THIS_DIR, "quickbench.py")
MEMBENCH = os.path.join(THIS_DIR, "membench.py")

# A wrapper (run with system python3) that executes a command and reports
# wall time + child peak RSS + return code as one JSON line on stdout.
WRAPPER = r"""
import subprocess, sys, time, resource, json
t0 = time.monotonic()
r = subprocess.run(sys.argv[1:], capture_output=True, text=True, timeout=600)
t1 = time.monotonic()
u = resource.getrusage(resource.RUSAGE_CHILDREN)
print(json.dumps({
    "time": round(t1 - t0, 6),
    "rss_kb": u.ru_maxrss,
    "retcode": r.returncode,
    "stdout_tail": r.stdout[-300:],
    "stderr_tail": r.stderr[-300:],
}))
"""


def run_measured(interp, script):
    """Run `interp script`, return (time_s, rss_kb, parsed_lines_or_None)."""
    out = subprocess.run(
        [sys.executable, "-c", WRAPPER, interp, script],
        capture_output=True, text=True,
    )
    try:
        m = json.loads(out.stdout.strip().splitlines()[-1])
    except Exception:
        return None, None, None
    if m["retcode"] != 0:
        return m["time"], m["rss_kb"], None
    return m["time"], m["rss_kb"], m["stdout_tail"]


def parse_quickbench(stdout_tail):
    rows = {}
    for line in stdout_tail.splitlines():
        parts = line.split("\t")
        if len(parts) == 3:
            try:
                rows[parts[0]] = float(parts[1])
            except ValueError:
                pass
    return rows


def git_commit():
    try:
        return subprocess.run(
            ["git", "-C", PROJECT, "rev-parse", "--short", "HEAD"],
            capture_output=True, text=True).stdout.strip()
    except Exception:
        return "unknown"


def suite_counts():
    """Reuse cpython_runner.sh over all test_*.py; return (pass, fail, timeout, parse)."""
    tests_dir = os.path.join(PROJECT, "tests", "cpython")
    scripts = sorted(os.path.join(tests_dir, f)
                     for f in os.listdir(tests_dir) if f.startswith("test_") and f.endswith(".py"))
    runner = os.path.join(PROJECT, "tests", "cpython_runner.sh")
    binary = os.path.join(PROJECT, "target", "debug", "rustpython")
    logdir = "/tmp/rustpython-trend-logs"
    os.makedirs(logdir, exist_ok=True)
    p = f = t = e = 0
    for script in scripts:
        r = subprocess.run(
            ["bash", runner, binary, script, logdir, "30", ""],
            capture_output=True, text=True, timeout=45,
        )
        line = r.stdout.strip().splitlines()[-1] if r.stdout.strip() else ""
        if line.startswith("PASS|"):
            p += 1
        elif line.startswith("FAIL|"):
            f += 1
        elif line.startswith("TIMEOUT|"):
            t += 1
        else:
            e += 1
    return p, f, t, e


def main():
    quick = "--quick" in sys.argv
    with_tests = "--tests" in sys.argv

    print("== RustPython vs CPython — trend measurement ==")
    print("commit:", git_commit())
    print("")

    # Quickbench on both interpreters.
    print("--- quickbench (seconds; lower is better) ---")
    cp = run_measured(CPYTHON, QUICKBENCH)
    rp = run_measured(RELEASE, QUICKBENCH)
    cp_rows = parse_quickbench(cp[2]) if cp[2] else {}
    rp_rows = parse_quickbench(rp[2]) if rp[2] else {}
    print("%-14s %12s %12s %8s" % ("bench", "cpython", "rustpython", "ratio"))
    row = {"commit": git_commit(), "timestamp": time.strftime("%Y-%m-%d %H:%M:%S")}
    for name in sorted(set(cp_rows) | set(rp_rows)):
        c = cp_rows.get(name)
        r = rp_rows.get(name)
        ratio = "n/a"
        if c and r:
            ratio = "%.2fx" % (r / c)
            row["bench_%s" % name] = r / c
        print("%-14s %12s %12s %8s" % (name,
              ("%.4f" % c) if c else "-",
              ("%.4f" % r) if r else "-", ratio))
    row["rss_quickbench_kb_cpython"] = cp[1]
    row["rss_quickbench_kb_rustpython"] = rp[1]

    print("")
    print("--- membench (peak RSS KB; lower is better) ---")
    cpm = run_measured(CPYTHON, MEMBENCH)
    rpm = run_measured(RELEASE, MEMBENCH)
    print("cpython   : %s KB" % cpm[1])
    print("rustpython: %s KB" % rpm[1])
    row["rss_membench_kb_cpython"] = cpm[1]
    row["rss_membench_kb_rustpython"] = rpm[1]
    if cpm[1] and rpm[1]:
        row["rss_membench_ratio"] = rpm[1] / cpm[1]

    if with_tests:
        print("")
        print("--- CPython suite aggregate (PASS/FAIL/TIMEOUT/ERROR) ---")
        p, f, t, e = suite_counts()
        print("PASS=%d FAIL=%d TIMEOUT=%d ERROR=%d" % (p, f, t, e))
        row["tests_pass"] = p
        row["tests_fail"] = f
        row["tests_timeout"] = t
        row["tests_error"] = e

    # Append to trend.json.
    os.makedirs(RESULTS_DIR, exist_ok=True)
    try:
        with open(TREND_FILE) as fh:
            trend = json.load(fh)
    except Exception:
        trend = []
    trend.append(row)
    with open(TREND_FILE, "w") as fh:
        json.dump(trend, fh, indent=1)
    print("")
    print("appended to %s (total rows: %d)" % (TREND_FILE, len(trend)))


if __name__ == "__main__":
    main()
