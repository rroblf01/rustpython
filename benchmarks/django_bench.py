"""RustPython Phase 0 Benchmark Suite.
Compares CPython vs RustPython performance.
Usage:
    python3 benchmarks/django_bench.py          # CPython
    rustpython benchmarks/django_bench.py       # RustPython
"""

import sys
import os

SITE_PACKAGES = '/tmp/rustpython-bench-env/lib/python3.14/site-packages'

if os.path.exists(SITE_PACKAGES):
    sys.path.insert(0, SITE_PACKAGES)

N = 20000

def time_it(fn):
    t0 = __import__('time').perf_counter()
    result = fn()
    t = __import__('time').perf_counter() - t0
    return t, result

BENCHMARKS = []

def register(name):
    def dec(fn):
        BENCHMARKS.append((name, fn))
        return fn
    return dec

@register("import_django")
def import_django():
    import django
    return django.VERSION

@register("attr_access")
def attr_access():
    class Vec:
        def __init__(self, x, y):
            self.x = x
            self.y = y
        def dot(self, other):
            return self.x * other.x + self.y * other.y
    v1 = Vec(3, 4)
    v2 = Vec(5, 6)
    r = 0
    for i in range(N):
        r += v1.dot(v2)
    return r

@register("dict_ops")
def dict_ops():
    d = {"a": 1, "b": 2}
    for i in range(N):
        d["c"] = i
        a = d.get("a")
        b = "a" in d
    return a

@register("function_call")
def function_call():
    def f(a, b, c, d):
        return a + b + c + d
    r = 0
    for i in range(N):
        r += f(i, i+1, i+2, i*2)
    return r

@register("tuple_build")
def tuple_build():
    r = 0
    for i in range(N):
        t = (i, i+1, i+2)
        r += t[0] + t[1] + t[2]
    return r

@register("list_append")
def list_append():
    lst = []
    for i in range(N):
        lst.append(i)
        lst.append(i+1)
    return len(lst)

@register("arith")
def arith():
    r = 0
    for i in range(N):
        r += i * 2
        r -= i // 3
        if i % 2 == 0:
            r += -i
        else:
            r += i
    return r

@register("fibonacci")
def fibonacci():
    r = 0
    a, b = 0, 1
    for i in range(25):
        a, b = b, a + b
    return a

@register("class_creation")
def class_creation():
    r = 0
    for i in range(5000):
        class A:
            pass
        a = A()
        a.x = i
        r += a.x
    return r

@register("string_format")
def string_format():
    r = ""
    for i in range(5000):
        r = f"hello_{i}_world_{i*2}"
    return len(r)

def main():
    has_time = False
    try:
        import time
        has_time = True
    except ImportError:
        pass

    results = []
    for name, fn in BENCHMARKS:
        if has_time:
            t, r = time_it(fn)
            print(f"{name}\t{t*1000:.1f}ms\t{r}")
            results.append((name, t, r))
        else:
            r = fn()
            print(f"{name}\t<time>\t{r}")
            results.append((name, None, r))

    if has_time:
        total = sum(t for _, t, _ in results if t is not None)
        print(f"\nTotal: {total*1000:.1f}ms")

main()
