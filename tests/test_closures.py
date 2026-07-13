# File: tests/test_closures.py
# Description: Tests for closures, nested functions, nonlocal, cell variables

print("=== SIMPLE CLOSURES ===")
def outer(x):
    def inner():
        return x
    return inner

f = outer(42)
result = f()
assert result == 42, f"Simple closure failed: {result}"

f2 = outer(99)
assert f2() == 99, f"Second closure instance failed: {f2()}"
# Each call creates a new closure
assert f() == 42, "First closure should still be 42"
print("OK")

print("=== CELL VARIABLE CAPTURE ===")
def make_counter():
    count = 0
    def increment():
        nonlocal count
        count = count + 1
        return count
    return increment

counter = make_counter()
assert counter() == 1, f"Counter first call: {counter()}"
# Wait - this will be 2 because we called counter() in the assert
# Let me redo this properly
a = make_counter()
assert a() == 1, f"Counter first: {a()}"
assert a() == 2, f"Counter second: {a()}"
assert a() == 3, f"Counter third: {a()}"

# Independent counters
b = make_counter()
assert b() == 1, f"Second counter first: {b()}"

print("OK")

print("=== MULTIPLE LEVELS OF NESTING === (basic)")
def level1(x):
    def level2(y):
        return x + y
    return level2

f = level1(10)
assert f(5) == 15, f"Two-level nesting failed: {f(5)}"
print("OK")
print("=== CLOSURES WITH DEFAULT ARGS ===")
def make_adder(base):
    def add(x, y=10, z=100):
        return base + x + y + z
    return add

adder = make_adder(1)
result = adder(2)
assert result == 113, f"Closure with defaults (no override): {result}"
result = adder(2, 20)
assert result == 123, f"Closure with defaults (y override): {result}"
result = adder(2, 20, 200)
assert result == 223, f"Closure with defaults (all override): {result}"
print("OK")

print("=== NONLOCAL ===")
def make_accumulator():
    total = 0
    def add(value):
        nonlocal total
        total = total + value
        return total
    return add

acc = make_accumulator()
assert acc(5) == 5, f"Nonlocal accumulator: {acc(5)}"
# Redo without double-call
acc2 = make_accumulator()
assert acc2(5) == 5, f"Accumulator first: {acc2(5)}"
assert acc2(3) == 8, f"Accumulator second: {acc2(3)}"
assert acc2(2) == 10, f"Accumulator third: {acc2(2)}"
print("OK")

print("=== GLOBAL VARIABLE USAGE ===")
# Test global keyword in nested context
GLOBAL_VAR = 10

def outer_global():
    def inner_global():
        global GLOBAL_VAR
        return GLOBAL_VAR
    return inner_global()

result = outer_global()
assert result == 10, f"Global in nested function: {result}"
print("OK")

print("=== CLOSURE IN LIST ===")
def make_functions():
    funcs = []
    def make_func(n):
        def f():
            return n
        return f
    for i in range(3):
        funcs.append(make_func(i))
    return funcs

funcs = make_functions()
assert funcs[0]() == 0, f"Closure list [0]: {funcs[0]()}"
assert funcs[1]() == 1, f"Closure list [1]: {funcs[1]()}"
assert funcs[2]() == 2, f"Closure list [2]: {funcs[2]()}"
print("OK")

print("=== ALL TESTS PASSED ===")
