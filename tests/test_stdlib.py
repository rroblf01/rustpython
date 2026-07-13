# File: tests/test_stdlib.py
# Description: Tests for standard library modules:
#   re, functools, __future__, atexit, logging

print("=== RE MODULE ===")
import re

# compile with raw string
c = re.compile(r"\d+")
assert c is not None, "re.compile returned None"

# match
m = c.match("123abc")
assert m is not None, "re.match returned None on matching input"
m = c.match("abc123")
assert m is None, "re.match should return None on non-matching input"

# search
s = c.search("abc123def")
assert s is not None, "re.search returned None"
s = c.search("abcdef")
assert s is None, "re.search should return None on non-matching"

# findall
result = c.findall("12 abc 34 def 56")
assert result == ["12", "34", "56"], f"re.findall failed: {result}"
result = c.findall("abc")
assert result == [], f"re.findall on non-matching failed: {result}"

# sub
result = c.sub("X", "12 abc 34")
assert result == "X abc X", f"re.sub failed: {result}"
result = c.sub("Y", "abc")
assert result == "abc", f"re.sub on non-matching failed: {result}"

# split
result = c.split("12abc34def56")
assert "abc" in result, f"re.split missing 'abc': {result}"
assert "def" in result, f"re.split missing 'def': {result}"

# pattern and flags
assert c.pattern == r"\d+", f"re.pattern failed: {c.pattern}"
assert c.flags == 0, f"re.flags failed: {c.flags}"

# re module functions
m2 = re.match(r"hello", "hello world")
assert m2 is not None, "re.match function failed"
result = re.findall(r"\w+", "a b c")
assert result == ["a", "b", "c"], f"re.findall function failed: {result}"

print("OK")

print("=== FUNCTOOLS MODULE ===")
import functools

# partial
def add(a, b):
    return a + b
add5 = functools.partial(add, 5)
result = add5(3)
assert result == 8, f"functools.partial failed: {result}"
result = add5(10)
assert result == 15, f"functools.partial second call failed: {result}"

# reduce
result = functools.reduce(lambda a, b: a + b, [1, 2, 3, 4])
assert result == 10, f"functools.reduce failed: {result}"
result = functools.reduce(lambda a, b: a * b, [1, 2, 3, 4])
assert result == 24, f"functools.reduce multiplication failed: {result}"
result = functools.reduce(lambda a, b: a + b, [5])
assert result == 5, f"functools.reduce single element failed: {result}"

# wraps
from functools import wraps

def decorator(f):
    @wraps(f)
    def wrapper(x):
        return f(x)
    return wrapper

@decorator
def my_func(x):
    return x + 1

result = my_func(10)
assert result == 11, "wraps decorated function failed"
assert my_func.__name__ == "my_func", "wraps __name__ failed"

# update_wrapper directly
def target_func():
    pass

def wrapper_func():
    pass

functools.update_wrapper(wrapper_func, target_func)
assert wrapper_func.__name__ == "target_func", f"update_wrapper failed: {wrapper_func.__name__}"

print("OK")

print("=== __FUTURE__ MODULE ===")
import __future__

# Check that basic attributes exist
assert hasattr(__future__, "annotations"), "__future__ missing annotations"
assert hasattr(__future__, "division"), "__future__ missing division"
assert hasattr(__future__, "print_function"), "__future__ missing print_function"
assert hasattr(__future__, "generators"), "__future__ missing generators"
assert hasattr(__future__, "nested_scopes"), "__future__ missing nested_scopes"
assert hasattr(__future__, "absolute_import"), "__future__ missing absolute_import"
assert hasattr(__future__, "unicode_literals"), "__future__ missing unicode_literals"
assert hasattr(__future__, "with_statement"), "__future__ missing with_statement"
assert hasattr(__future__, "all_feature_names"), "__future__ missing all_feature_names"
assert hasattr(__future__, "__doc__"), "__future__ missing __doc__"
assert hasattr(__future__, "__name__"), "__future__ missing __name__"
assert hasattr(__future__, "__package__"), "__future__ missing __package__"

print("OK")

print("=== ATEXIT MODULE ===")
import atexit

# Test register with a simple lambda
atexit.register(lambda: None)
print("atexit.register: OK")

# Test unregister
def cleanup_func():
    pass

atexit.register(cleanup_func)
atexit.unregister(cleanup_func)
print("atexit.unregister: OK")

print("OK")

print("=== LOGGING MODULE ===")
import logging

# NullHandler
nh = logging.NullHandler()
assert isinstance(nh, logging.Handler), "NullHandler not a Handler instance"
assert nh.level == logging.NOTSET, f"NullHandler level: {nh.level}"

# Logger with NullHandler
logger = logging.getLogger("test")
logger.addHandler(nh)
logger.setLevel(logging.DEBUG)
logger.debug("debug message")
logger.info("info message")
logger.warning("warning message")
logger.error("error message")
print("logging with NullHandler: OK")

# logging constants
assert logging.DEBUG == 10, f"DEBUG constant: {logging.DEBUG}"
assert logging.INFO == 20, f"INFO constant: {logging.INFO}"
assert logging.WARNING == 30, f"WARNING constant: {logging.WARNING}"
assert logging.ERROR == 40, f"ERROR constant: {logging.ERROR}"
assert logging.CRITICAL == 50, f"CRITICAL constant: {logging.CRITICAL}"
assert logging.NOTSET == 0, f"NOTSET constant: {logging.NOTSET}"

# basicConfig
logging.basicConfig()

print("OK")

print("=== ALL TESTS PASSED ===")
