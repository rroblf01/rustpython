# Comprehensive CPython 3.14 compatibility test
# Tests key features that must work for 100% compatibility

import sys

print("=== Version ===")
print(f"sys.version = {sys.version}")

print("\n=== 1. Basic Types and Operations ===")

# Integers
assert 2 ** 2000 > 10 ** 100  # big ints
assert -3 // 2 == -2  # floor division
assert int("deadbeef", 16) == 0xdeadbeef
assert int.bit_length(255) == 8

# Strings
assert "hello".capitalize() == "Hello"
assert "hello".center(11) == "   hello   "
assert "hello".count("l") == 2
assert "hello".endswith("lo")
assert "hello".find("l") == 2
assert "hello".index("l") == 2
assert "hello".isalnum()
assert "hello".isalpha()
assert "hello".isdigit() == False
assert "hello".islower()
assert "hello".isupper() == False
assert "hello".join(["a", "b"]) == "ahellob"
assert "hello".ljust(10) == "hello     "
assert "hello".lower() == "hello"
assert "  hello  ".strip() == "hello"
assert "  hello  ".lstrip() == "hello  "
assert "  hello  ".rstrip() == "  hello"
assert "hello".replace("l", "x") == "hexxo"
assert "hello".rfind("l") == 3
assert "a,b,c".split(",") == ["a", "b", "c"]
assert "a\nb\nc".splitlines() == ["a", "b", "c"]
assert "hello".startswith("he")
assert "  hello  ".strip() == "hello"
assert "hello".swapcase() == "HELLO"
assert "hello".title() == "Hello"
assert "hello".upper() == "HELLO"
assert "hello".zfill(10) == "00000hello"
print("str methods: OK")

# Bytes
b = b"hello"
assert b.hex() == "68656c6c6f"
assert b.decode() == "hello"
assert bytes([104, 101, 108, 108, 111]) == b"hello"
print("bytes: OK")

# Lists
l = [1, 2, 3]
l.insert(0, 0)
assert l == [0, 1, 2, 3]
l.remove(1)
assert l == [0, 2, 3]
assert l.pop() == 3
l.clear()
assert l == []
l.extend([1, 2])
assert l == [1, 2]
l.append(3)
assert l == [1, 2, 3]
print("list methods: OK")

# Dicts
d = {"a": 1, "b": 2}
assert d.pop("a") == 1
assert d == {"b": 2}
d.update({"c": 3})
assert d == {"b": 2, "c": 3}
d.setdefault("d", 4)
assert d["d"] == 4
d.setdefault("b", 99)
assert d["b"] == 2
assert list(d.keys()) == ["b", "c", "d"]
assert list(d.values()) == [2, 3, 4]
d.popitem()
assert len(d) == 2
print("dict methods: OK")

# Sets
s = {1, 2, 3}
s.add(4)
assert s == {1, 2, 3, 4}
s.discard(5)  # should not raise
s.discard(4)
assert 4 not in s
s.update({5, 6})
assert s == {1, 2, 3, 5, 6}
s.intersection_update({1, 2})
assert s == {1, 2}
print("set methods: OK")

print("\n=== 2. Advanced Features ===")

# Properties
class PropTest:
    @property
    def x(self):
        return 42
    @x.setter
    def x(self, val):
        self._x = val
    @x.deleter
    def x(self):
        self._x = None
p = PropTest()
assert p.x == 42
p.x = 100
assert p._x == 100
print("property: OK")

# Classmethod / Staticmethod
class CM:
    @classmethod
    def clsmeth(cls):
        return cls
    @staticmethod
    def statmeth():
        return 42
assert CM.clsmeth() == CM
assert CM.statmeth() == 42
assert CM().clsmeth() == CM
assert CM().statmeth() == 42
print("classmethod/staticmethod: OK")

# Super
class Base:
    def method(self):
        return "Base"
class Derived(Base):
    def method(self):
        return "Derived:" + super().method()
d = Derived()
assert d.method() == "Derived:Base"
print("super: OK")

# Descriptors
class Desc:
    def __get__(self, obj, objtype):
        return 42
    def __set__(self, obj, val):
        obj._val = val
    def __delete__(self, obj):
        obj._val = None
class DescOwner:
    attr = Desc()
o = DescOwner()
assert o.attr == 42
o.attr = 100
assert o._val == 100
del o.attr
assert o._val is None
print("descriptors: OK")

# Generators with .send(), .throw(), .close()
def gen_send():
    val = yield 1
    yield val
g = gen_send()
assert next(g) == 1
assert g.send(42) == 42
print("generator.send: OK")

# Generator throw
def gen_throw():
    try:
        yield 1
    except ValueError:
        yield "caught"
g = gen_throw()
next(g)
assert g.throw(ValueError) == "caught"
print("generator.throw: OK")

# Generator close
def gen_close():
    try:
        yield 1
    finally:
        pass  # just testing we don't crash
g = gen_close()
next(g)
g.close()
print("generator.close: OK")

# Context managers (with statement)
class MyCM:
    def __enter__(self):
        return 42
    def __exit__(self, *args):
        return False
with MyCM() as x:
    assert x == 42
print("context manager: OK")

# Iteration protocol - __iter__ returning self
class MyIter:
    def __init__(self, n):
        self.n = n
        self.i = 0
    def __iter__(self):
        return self
    def __next__(self):
        if self.i >= self.n:
            raise StopIteration
        self.i += 1
        return self.i
assert list(MyIter(3)) == [1, 2, 3]
print("__iter__/__next__ protocol: OK")

# Multiple inheritance MRO
class A:
    def method(self):
        return "A"
class B(A):
    def method(self):
        return "B"
class C(A):
    def method(self):
        return "C"
class D(B, C):
    pass
d = D()
assert d.method() == "B"  # MRO should be D -> B -> C -> A
print("multiple inheritance MRO: OK")

print("\n=== 3. Exception Handling ===")

# Nested exceptions with chaining
try:
    try:
        raise ValueError("inner")
    except ValueError as e:
        raise RuntimeError("outer") from e
except RuntimeError as e:
    assert isinstance(e.__cause__, ValueError)
    assert str(e.__cause__) == "inner"
print("exception chaining: OK")

# Finally with return
def finally_return():
    try:
        return 1
    finally:
        return 2  # overrides
assert finally_return() == 2
print("finally overriding return: OK")

# Exception group (PEP 654)
try:
    exec("raise ExceptionGroup('group', [ValueError('a')])")
    print("ExceptionGroup: OK")
except NameError:
    print("ExceptionGroup: not implemented")

print("\n=== 4. Standard Library Modules ===")

# Check key modules exist
modules_to_check = [
    "math", "sys", "os", "re", "json", "collections", "datetime",
    "random", "itertools", "functools", "statistics", "enum",
    "decimal", "fractions", "types", "copy", "struct", "textwrap",
    "pprint", "string", "pathlib", "shutil", "glob", "tempfile",
    "uuid", "csv", "io", "threading", "queue", "select",
    "socket", "subprocess", "hashlib", "base64", "zlib",
    "argparse", "configparser", "pickle", "heapq", "bisect",
    "logging", "locale", "numbers", "ast", "abc", "typing",
    "dataclasses", "unittest", "dis", "doctest", "imghdr",
    "getopt", "getpass", "platform", "weakref", "array",
    "smtplib", "http.client", "xml.etree.ElementTree",
]
missing_modules = []
for mod_name in modules_to_check:
    try:
        __import__(mod_name)
    except ImportError:
        missing_modules.append(mod_name)

if missing_modules:
    print(f"Missing modules ({len(missing_modules)}): {missing_modules}")
else:
    print("All checked modules: OK")

print("\n=== 5. Syntax Features ===")

# Walrus operator nested
data = [1, 2, 3, 4]
filtered = [y for x in data if (y := x * 2) > 4]
assert filtered == [6, 8]
print("nested walrus: OK")

# Match with complex patterns
def match_complex(val):
    match val:
        case {"key": v}:
            return f"dict:{v}"
        case [a, b, *rest]:
            return f"list:{a},{b},{rest}"
        case _:
            return "other"
assert match_complex({"key": 42}) == "dict:42"
assert match_complex([1, 2, 3, 4]) == "list:1,2,[3, 4]"
print("match complex patterns: OK")

# Formatted string literals with expressions
x = 42
assert f"{x} + 1 = {x + 1}" == "42 + 1 = 43"
assert f"{x:10d}" == "        42"
assert f"{x:<10d}" == "42        "
print("f-string expressions and format specs: OK")

print("\n=== 6. File Operations ===")
with open("/dev/null", "r") as f:
    assert f.closed == False
assert f.closed == True
print("file context manager: OK")

# Tempfile via builtins
with open("tests/_tmp_test.txt", "w") as f:
    f.write("hello world")
with open("tests/_tmp_test.txt", "r") as f:
    data = f.read()
assert data == "hello world"
import os
os.unlink("tests/_tmp_test.txt")
print("file read/write: OK")

print("\n=== PHASE 1 COMPLETE ===")
