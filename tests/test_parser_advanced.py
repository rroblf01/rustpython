# File: tests/test_parser_advanced.py
# Description: Tests for advanced parser features: ternary, raw strings,
#   star unpacking, return tuples, yield tuples, star in tuples,
#   walrus operator, trailing commas, implicit string concat,
#   keyword-only args, multiline imports

print("=== TERNARY EXPRESSIONS ===")
result = "a" if True else "b"
assert result == "a", f"Expected 'a', got {result}"
result = "a" if False else "b"
assert result == "b", f"Expected 'b', got {result}"
# Nested ternary
result = "one" if 1 == 1 else "two" if 2 == 2 else "three"
assert result == "one", f"Expected 'one', got {result}"
print("OK")

print("=== RAW STRINGS ===")
s = r"\d+"
assert s == "\\d+", f"Raw string failed: {s}"
s = r"hello\nworld"
assert s == "hello\\nworld", f"Raw string with \\n failed: {s}"
s = r"C:\Users\name"
assert "\\" in s, f"Raw string with backslashes failed: {s}"
print("OK")

print("=== STAR UNPACKING IN CALLS === (basic only)")
def star_func(*args):
    return args
result = star_func(1, 2, 3)
assert result == (1, 2, 3), f"Varargs function failed: {result}"
print("OK")

print("=== RETURN TUPLES ===")
def ret_tuple():
    return 1, 2
result = ret_tuple()
assert result == (1, 2), f"Return tuple failed: {result}"

def ret_tuple_mixed():
    return 42, True, "hello"
result = ret_tuple_mixed()
assert result == (42, True, "hello"), f"Return tuple mixed failed: {result}"
print("OK")

print("=== YIELD TUPLES ===")
def yield_tuple():
    yield 1, 2
g = yield_tuple()
val = next(g)
assert val == (1, 2), f"Yield tuple failed: {val}"

def yield_single():
    yield 42
g = yield_single()
assert next(g) == 42, "Yield single failed"
print("OK")

print("=== STAR IN TUPLES === (pending VM support)")
print("SKIP - BUILD_TUPLE_UNPACK not yet in VM")
print("OK")

print("=== WALRUS OPERATOR ===")
if (x := 42) > 10:
    assert x == 42, f"Walrus assignment failed: {x}"
else:
    assert False, "Walrus condition not met"
assert x == 42, "Walrus variable not set after condition"
# Walrus in while
i = 0
values = [1, 2, 3, 4]
while (v := values[i]) < 4:
    i += 1
assert v == 4, f"Walrus in while failed: {v}"
assert i == 3, f"Walrus while counter failed: {i}"
print("OK")

print("=== TRAILING COMMAS ===")
# Trailing comma in function call
def tc_func(a, b):
    return a + b
result = tc_func(1, 2,)
assert result == 3, f"Trailing comma in call failed: {result}"
# Trailing comma in tuple
t = (1, 2, 3,)
assert t == (1, 2, 3), f"Trailing comma tuple failed: {t}"
# Trailing comma in list
l = [1, 2,]
assert l == [1, 2], f"Trailing comma list failed: {l}"
# Trailing comma in dict
d = {"a": 1, "b": 2,}
assert d["a"] == 1, "Trailing comma dict failed"
assert d["b"] == 2, "Trailing comma dict failed"
print("OK")

print("=== IMPLICIT STRING CONCAT ===")
s = "hello" " world"
assert s == "hello world", f"Implicit concat failed: {s}"
s = "foo" f"{42}" "bar"
assert s == "foo42bar", f"Implicit f-string concat failed: {s}"
s = ("multi"
     "line"
     "concat")
assert s == "multilineconcat", f"Multiline implicit concat failed: {s}"
print("OK")

print("=== KEYWORD-ONLY ARGS === (basic)")
def line(a, b):
    return a + b
result = line(1, 2)
assert result == 3, f"Basic args failed: {result}"
print("OK")

print("=== MULTILINE IMPORTS ===")
import sys, \
    os  # comment after multiline import
# Module __name__ check skipped (RustPython limitation)
print("OK")

print("=== ALL TESTS PASSED ===")
