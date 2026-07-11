#!/usr/bin/env python3
"""Minimal benchmark for RustPython — no imports needed."""

N = 50000

# Arithmetic
n = 0
for i in range(N):
    n += i
    n -= i // 2
    n *= 2
    n //= 3
    n %= 1000
print("arithmetic:", n)

# Global lookup
RUNS = N
x = 0
for i in range(RUNS):
    x = RUNS
    y = True
print("global_lookup:", x, y)

# List ops
lst = [1, 2, 3]
for i in range(N):
    lst.append(i)
    a = lst[0]
    b = lst[-1]
    c = len(lst)
print("list_ops:", a, b, c)

# Dict ops
d = {"a": 1, "b": 2}
for i in range(N):
    d["c"] = i
    a = d.get("a")
    b = "a" in d
    c = len(d)
print("dict_ops:", a, b, c)

# Function call
def f(x):
    return x + 1

x = 0
for i in range(N):
    x = f(i)
print("function_call:", x)

# While loop
i = 0
n = 0
while i < N:
    n += i
    i += 1
print("while_loop:", n)

print("ALL BENCHMARKS DONE")
