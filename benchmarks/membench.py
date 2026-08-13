#!/usr/bin/env python3
"""Memory-focused workload: builds a large object graph and holds it.

Runs under BOTH RustPython and CPython. `benchmarks/bench_trend.py` measures
peak RSS while this runs.
"""

import time

# 200k elements: ints, small lists, small dicts, short strings.
lst = []
for i in range(50000):
    lst.append(i)
    lst.append([i, i + 1, i + 2])
    lst.append({"k%d" % i: i})
    lst.append("str%d" % i)

# A nested structure (list of lists of list/dict leaves).
outer = []
for i in range(5000):
    inner = []
    for j in range(10):
        inner.append([i * j, {"a": i, "b": j}])
    outer.append(inner)

time.sleep(0.1)
print("done", len(lst), len(outer))
