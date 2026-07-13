# RustPython Test Plan

**Generated**: July 13, 2026  
**Project**: `/opt/data/proyectos/rustpython/`  
**Target**: CPython 3.14 compatibility  
**Binary**: `target/debug/rustpython`  
**Test runner**: `make test-python` (executes all `tests/*.py` files in sorted order)

---

## Table of Contents

1. [Current Test Coverage](#section-1-current-test-coverage)
2. [Coverage Gaps by Domain](#section-2-coverage-gaps-by-domain)
3. [Proposed Test Files (Prioritized)](#section-3-proposed-test-files-prioritized)
4. [Python Flows Not Yet Covered](#section-4-python-flows-not-yet-covered)
5. [Known VM Gaps (Blockers)](#section-5-known-vm-gaps-blockers)
6. [Recommended Implementation Order](#section-6-recommended-implementation-order)

---

## Section 1: Current Test Coverage

### 1.1 Existing Test Files

| # | File | Lines | Focus | Assertions | Status |
|---|------|-------|-------|------------|--------|
| 1 | `test_basic.py` | 170 | Arithmetic, booleans, lists, strings, dicts, functions, lambdas, classes, isinstance, comprehensions, generators, genexprs, match/case, `__str__`, `__len__`, try/except, docstrings, walrus, *args, **kwargs, defaults, mixed args | ~30 assert | ✅ PASS |
| 2 | `test_completo.py` | 128 | f-string format specs, repr(exceptions), print sep/end, `str.__format__`, bytes.hex/decode, native modules (_thread, signal, gc, sysconfig, linecache), match first-statement, for-else, while-else, augmented assignment | ~20 assert | ✅ PASS |
| 3 | `test_closures.py` | 117 | Simple closures, cell variable capture, multi-level nesting, closures with defaults, nonlocal, global in nested functions, closures in list | ~15 assert | ✅ PASS |
| 4 | `test_hmac.py` | 6 | hmac import and basic HMAC creation | 0 assert | ✅ PASS |
| 5 | `test_os.py` | 9 | os.path.abspath, os.getcwd | 0 assert | ✅ PASS |
| 6 | `test_parser_advanced.py` | 119 | Ternary, raw strings, star unpacking (basic), return tuples, yield tuples, walrus operator, trailing commas, implicit string concat, multiline imports | ~20 assert | ✅ PASS |
| 7 | `test_sec.py` | 4 | secrets.token_bytes | 0 assert | ✅ PASS |
| 8 | `test_stdlib.py` | 169 | **re**: compile/match/search/findall/sub/split; **functools**: partial/reduce/wraps/update_wrapper; **__future__**: attributes; **atexit**: register/unregister; **logging**: NullHandler, Logger, constants | ~30 assert | ✅ PASS |
| 9 | `test_tanda14.py` | 16 | Smoke tests: repr exceptions, print sep, f-string format, bytes hex, native module imports | 0 assert | ✅ PASS |

### 1.2 Summary of Current Coverage

```
Fully covered:      basic types, arithmetic, string methods, list/dict ops,
                    functions (def, lambda, *args, **kwargs, defaults),
                    classes (basic), comprehensions, generators (single-yield),
                    match/case, f-strings, try/except, closures, nonlocal,
                    decorators (@wraps), re (basic), functools (partial/reduce/wraps),
                    __future__, atexit, logging (NullHandler), native module imports,
                    ternary, raw strings, walrus operator, trailing commas

Partially covered:  generators (multi-yield has known bug), os.path (abspath only),
                    bytes ops (hex/decode only), match (basic patterns only)

Not covered at all: see Section 2
```

---

## Section 2: Coverage Gaps by Domain

### 2.1 Parser / Syntax Gaps

| Feature | Covered? | Existing Test |
|---------|----------|---------------|
| Arithmetic operators (`+`, `-`, `*`, `/`, `//`, `%`, `**`) | ✅ Basic | test_basic.py |
| Bitwise operators (`\|`, `^`, `&`, `<<`, `>>`, `~`) | ❌ **None** | — |
| Comparison chaining (`a < b < c`) | ❌ **None** | — |
| `is` / `is not` / `in` / `not in` | ❌ **None** | — |
| Augmented assignment (`+=`, `-=`, etc.) | ✅ Single | test_completo.py |
| Walrus operator `:=` | ✅ | test_basic.py, test_parser_advanced.py |
| Ternary `x if cond else y` | ✅ | test_parser_advanced.py |
| Raw strings `r"..."` | ✅ | test_parser_advanced.py |
| f-string format specs | ✅ Basic | test_completo.py |
| f-string advanced (nested, expr, =) | ❌ **None** | — |
| Implicit string concat | ✅ | test_parser_advanced.py |
| Trailing commas in calls/lists/dicts | ✅ | test_parser_advanced.py |
| `match`/`case` with complex patterns | ⚠️ Basic only | test_basic.py |
| `match`/`case` with sequences, mappings, guards | ❌ **None** | — |
| `type` statement (PEP 695) | ❌ **None** | — |
| `except*` | ❌ **None** | — |
| `async def` / `await` | ❌ **None** (parsed but untested at runtime) | — |
| `async for` / `async with` | ❌ **None** | — |
| Keyword-only args (`def f(*, a)`) | ✅ Basic | test_parser_advanced.py |
| Positional-only args (`def f(a, /)`) | ❌ **None** | — |
| `*args` / `**kwargs` in function defs | ✅ | test_basic.py |
| `*args` / `**kwargs` in calls | ❌ **Blocked** (VM gap) | test_parser_advanced.py |

### 2.2 Type Operations Gaps

| Operation | Covered? | Notes |
|-----------|----------|-------|
| **int**: `int()`, bit ops, conversions | ❌ **None** | Basic arithmetic only |
| **float**: `float()`, `is_integer()`, rounding | ❌ **None** | — |
| **complex**: creation, operations | ❌ **None** | ConstValue::Complex exists |
| **bool**: `bool()`, short-circuit | ✅ Basic | test_basic.py |
| **str**: `.split()`, `.join()`, `.strip()`, `.find()`, `.index()`, `.startswith()`, `.endswith()`, `.replace()` | ⚠️ Partial | `.upper()`, `.replace()` tested |
| **str**: `.encode()`, `.format()`, `.partition()` | ❌ **None** | — |
| **bytes**: `.hex()`, `.decode()` | ✅ | test_completo.py |
| **bytes**: `.fromhex()`, `.split()`, constructor | ❌ **None** | — |
| **list**: `.append()`, `.pop()`, `.reverse()` | ✅ Basic | test_basic.py |
| **list**: `.sort()`, `.insert()`, `.remove()`, `.count()`, `.index()`, `.extend()`, slice assignment | ❌ **None** | — |
| **dict**: `.get()` | ✅ | test_basic.py |
| **dict**: `.keys()`, `.values()`, `.items()`, `.update()`, `.pop()`, `.setdefault()`, `.clear()` | ❌ **None** | — |
| **tuple**: creation, indexing, count, index | ❌ **None** | — |
| **set**: `set()`, `.add()`, `.remove()`, `.union()`, `.intersection()`, `.difference()` | ❌ **None** | — |
| slice objects: `obj[start:stop:step]` | ❌ **None** | BUILD_SLICE exists in VM |
| `Ellipsis` / `NotImplemented` | ❌ **None** | — |
| `None` comparisons and identity | ❌ **None** | — |
| Type constructors: `int(x)`, `str(x)`, `list(x)`, `tuple(x)`, `set(x)`, `dict(x)` | ❌ **None** | — |
| `range()`: creation, iteration, `len()`, indexing | ❌ **None** | RangeIter exists in VM |
| `enumerate()`: basic and with start | ❌ **None** | — |
| `zip()`: basic, with `strict=` | ❌ **None** | — |
| `map()`, `filter()`, `reversed()`, `sorted()` | ❌ **None** | — |

### 2.3 Function Gaps

| Feature | Covered? | Notes |
|---------|----------|-------|
| `def` with all arg types | ✅ Basic | test_basic.py |
| `lambda` | ✅ Basic | test_basic.py |
| `*args` / `**kwargs` | ✅ Basic | test_basic.py |
| Default arguments | ✅ Basic | test_basic.py |
| Closures | ✅ | test_closures.py |
| `nonlocal` | ✅ | test_closures.py |
| `global` | ✅ | test_closures.py |
| Nested functions | ✅ | test_closures.py |
| Decorators (`@decorator`) | ⚠️ Partial | `@wraps` only (test_stdlib.py) |
| Multiple decorators on one function | ❌ **None** | — |
| Decorators with arguments | ❌ **None** | — |
| `@staticmethod` | ❌ **None** | Features exist in VM |
| `@classmethod` | ❌ **None** | Features exist in VM |
| `@property` with getter/setter/deleter | ❌ **None** | Features exist in VM |
| `@functools.lru_cache` | ❌ **None** | Module function exists? |
| Recursion (depth limit, tail call) | ❌ **None** | — |
| `return` with no value | ❌ **None** | — |
| `yield from` | ❌ **None** | — |
| Generator `.send()` / `.throw()` / `.close()` | ❌ **None** | VM has Send opcode |

### 2.4 Class / OOP Gaps

| Feature | Covered? | Notes |
|---------|----------|-------|
| Basic class with methods | ✅ | test_basic.py |
| `__init__` constructor | ❌ **None** | — |
| `__str__` | ✅ | test_basic.py |
| `__repr__` | ❌ **None** | — |
| `__len__` | ✅ | test_basic.py |
| `__getattr__` / `__setattr__` / `__getattribute__` | ❌ **None** | VM supports these |
| `__delattr__` | ❌ **None** | — |
| `__call__` | ❌ **None** | — |
| `__iter__` / `__next__` | ✅ | GAP_ANALYSIS says fixed |
| `__enter__` / `__exit__` (context managers) | ❌ **None** | `with` statement tested? |
| `__contains__` | ❌ **None** | CONTAINS_OP in VM |
| `__add__`, `__radd__`, etc. (operator overloading) | ❌ **None** | — |
| `__bool__` / `__nonzero__` | ❌ **None** | — |
| `__hash__` | ❌ **None** | — |
| `__eq__` / `__ne__` / `__lt__` etc. | ❌ **None** | — |
| Single inheritance | ❌ **None** | — |
| Multiple inheritance / MRO | ❌ **None** | C3 linearization implemented |
| `super()` without arguments | ❌ **None** | VM has support |
| `super(Type, self)` with arguments | ❌ **None** | May be blocked |
| `isinstance()` / `issubclass()` | ⚠️ Basic isinstance | test_basic.py |
| `__slots__` | ❌ **None** | extract_slots exists in object.rs |
| `@dataclass` | ❌ **None** | Native module exists |
| `@staticmethod` / `@classmethod` decorators | ❌ **None** | VM has StaticMethod/ClassMethod |
| Abstract base classes (`abc.ABC`) | ❌ **None** | Native abc module exists |
| `@property` | ❌ **None** | VM has Property object |
| Descriptor protocol (`__get__`, `__set__`, `__delete__`) | ❌ **None** | VM implements these |
| Type creation (`type(name, bases, dict)`) | ❌ **None** | — |
| Metaclasses (`__metaclass__`) | ❌ **None** | — |

### 2.5 Stdlib Module Gaps

| Module | Covered? | What's Tested | Native Module Status |
|--------|----------|---------------|---------------------|
| `re` | ⚠️ Partial | compile/match/search/findall/sub/split | ✅ Native |
| `functools` | ⚠️ Partial | partial/reduce/wraps/update_wrapper | ✅ Native |
| `__future__` | ✅ | All feature names | ✅ Native |
| `atexit` | ✅ | register/unregister | ✅ Native |
| `logging` | ⚠️ Partial | NullHandler, Logger, constants | ✅ Native |
| `os` | ⚠️ Minimal | os.path.abspath, os.getcwd | ✅ Native |
| `hmac` | ⚠️ Minimal | HMAC creation, hexdigest | ✅ Native |
| `secrets` | ⚠️ Minimal | token_bytes | ✅ Native |
| `json` | ❌ **None** | — | ✅ Native |
| `math` | ❌ **None** | — | ✅ Native |
| `sys` | ❌ **None** | — | ✅ Native |
| `collections` | ❌ **None** | — | ✅ Native |
| `itertools` | ❌ **None** | — | ✅ Native |
| `random` | ❌ **None** | — | ✅ Native |
| `datetime` | ❌ **None** | — | ✅ Native |
| `copy` | ❌ **None** | — | ✅ Native |
| `types` | ❌ **None** | — | ✅ Native |
| `enum` | ❌ **None** | — | ✅ Native |
| `struct` | ❌ **None** | — | ✅ Native |
| `bisect` | ❌ **None** | — | ✅ Native |
| `heapq` | ❌ **None** | — | ✅ Native |
| `pathlib` | ❌ **None** | — | ✅ Native |
| `io` (StringIO) | ❌ **None** | — | ✅ Native |
| `hashlib` | ❌ **None** | — | ✅ Native |
| `base64` | ❌ **None** | — | ✅ Native |
| `uuid` | ❌ **None** | — | ✅ Native |
| `subprocess` | ❌ **None** | — | ✅ Native |
| `threading` | ❌ **None** | — | ✅ Native |
| `socket` | ❌ **None** | — | ✅ Native |
| `decimal` | ❌ **None** | — | ✅ Native |
| `fractions` | ❌ **None** | — | ✅ Native |
| `statistics` | ❌ **None** | — | ✅ Native |
| `contextlib` | ❌ **None** | — | ✅ Native |
| `ast` (literal_eval) | ❌ **None** | — | ✅ Native |
| `dis` | ❌ **None** | — | ✅ Native |
| `typing` | ❌ **None** | — | ✅ Native (stubs) |
| `dataclasses` | ❌ **None** | — | ✅ Native (partial) |
| `inspect` | ❌ **None** | — | ✅ Native |
| `traceback` | ❌ **None** | — | ✅ Native (minimal) |
| `pprint` | ❌ **None** | — | ✅ Native |
| `textwrap` | ❌ **None** | — | ✅ Native |
| `shutil` | ❌ **None** | — | ✅ Native |
| `tempfile` | ❌ **None** | — | ✅ Native |
| `glob` | ❌ **None** | — | ✅ Native |
| `fnmatch` | ❌ **None** | — | ✅ Native |
| `operator` | ❌ **None** | — | ✅ Native |
| `string` | ❌ **None** | — | ✅ Native |
| `csv` | ❌ **None** | — | ✅ Native |
| `difflib` | ❌ **None** | — | ✅ Native |
| `gzip` | ❌ **None** | — | ✅ Native |
| `zlib` | ❌ **None** | — | ✅ Native |
| `zipfile` | ❌ **None** | — | ✅ Native (read-only) |
| `tarfile` | ❌ **None** | — | ✅ Native |
| `unittest` | ❌ **None** | — | ✅ Native (stubs) |
| `argparse` | ❌ **None** | — | ✅ Native (stubs) |
| `http.client` | ❌ **None** | — | ✅ Native |
| `smtplib` | ❌ **None** | — | ✅ Native (stub) |
| `html` | ❌ **None** | — | ✅ Native |
| `warnings` | ❌ **None** | — | ✅ Native |
| `asyncio` | ❌ **None** | — | ✅ Native (basic event loop) |
| `pickle` | ❌ **None** | — | ✅ Native (minimal stub) |

### 2.6 Built-in Function Gaps

| Built-in | Covered? | Notes |
|----------|----------|-------|
| `abs()`, `all()`, `any()` | ❌ **None** | — |
| `bin()`, `oct()`, `hex()` | ❌ **None** | — |
| `bool()` | ❌ **None** | — |
| `bytes()` | ❌ **None** | — |
| `callable()` | ❌ **None** | — |
| `chr()`, `ord()` | ❌ **None** | — |
| `classmethod()` | ❌ **None** | — |
| `delattr()` | ❌ **None** | VM has DELETE_ATTR |
| `dict()` | ❌ **None** | — |
| `dir()` | ❌ **None** | — |
| `divmod()` | ❌ **None** | — |
| `enumerate()` | ❌ **None** | EnumerateIter exists in VM |
| `eval()` | ❌ **None** | — |
| `exec()` | ❌ **None** | VM supports (reuses VM) |
| `filter()` | ❌ **None** | — |
| `float()` | ❌ **None** | — |
| `format()` | ✅ Basic | str.__format__ tested |
| `frozenset()` | ❌ **None** | — |
| `getattr()` | ❌ **None** | — |
| `globals()` | ❌ **None** | — |
| `hasattr()` | ❌ **None** | — |
| `hash()` | ❌ **None** | — |
| `help()` | ❌ **None** | — |
| `hex()` | ❌ **None** | — |
| `id()` | ❌ **None** | — |
| `input()` | ❌ **None** | — |
| `int()` | ❌ **None** | — |
| `isinstance()` | ✅ Basic | test_basic.py |
| `issubclass()` | ❌ **None** | — |
| `iter()` | ❌ **None** | GET_ITER in VM |
| `len()` | ✅ | test_basic.py (via __len__) |
| `list()` | ❌ **None** | — |
| `locals()` | ❌ **None** | — |
| `map()` | ❌ **None** | — |
| `max()`, `min()` | ❌ **None** | — |
| `next()` | ❌ **None** | FOR_ITER in VM |
| `object()` | ❌ **None** | — |
| `open()` | ❌ **None** | — |
| `pow()` | ❌ **None** | — |
| `print()` | ✅ Basic | sep/end in test_completo |
| `property()` | ❌ **None** | VM has Property |
| `range()` | ❌ **None** | RangeIter in VM |
| `repr()` | ✅ Basic | repr(exception) tested |
| `reversed()` | ❌ **None** | — |
| `round()` | ❌ **None** | — |
| `set()` | ❌ **None** | — |
| `setattr()` | ❌ **None** | — |
| `slice()` | ❌ **None** | BUILD_SLICE in VM |
| `sorted()` | ❌ **None** | — |
| `staticmethod()` | ❌ **None** | VM has StaticMethod |
| `str()` | ❌ **None** | — |
| `sum()` | ❌ **None** | — |
| `super()` | ❌ **None** | — |
| `tuple()` | ❌ **None** | — |
| `type()` | ❌ **None** | — |
| `vars()` | ❌ **None** | Partial in object.rs |
| `zip()` | ❌ **None** | — |
| `__import__()` | ❌ **None** | — |

### 2.7 Error Handling Gaps

| Feature | Covered? | Notes |
|---------|----------|-------|
| `try/except` (basic) | ✅ | test_basic.py |
| `try/except` with `as e` | ❌ **None** | — |
| `try/except` multiple handlers | ❌ **None** | — |
| `try/except/else` | ❌ **None** | — |
| `try/except/finally` | ❌ **None** | VM has SETUP_FINALLY |
| `try/finally` (no except) | ❌ **None** | — |
| `raise` with no argument | ❌ **None** | — |
| `raise ... from` | ❌ **None** | — |
| `assert` | ❌ **None** | — |
| `ExceptionGroup` / `except*` | ❌ **None** | VM has CHECK_EXC_MATCH_STAR |
| `try/except*` | ❌ **None** | Parser gap (no ExceptStar) |
| `UserWarning`, `DeprecationWarning` | ❌ **None** | — |
| nested exception handling | ❌ **None** | — |

### 2.8 Import System Gaps

| Feature | Covered? | Notes |
|---------|----------|-------|
| `import module` | ⚠️ Smoke | test_completo, test_hmac, etc. |
| `from module import name` | ⚠️ Smoke | test_stdlib.py |
| Relative imports | ❌ **None** | — |
| Circular imports | ❌ **None** | — |
| Package `__init__.py` | ❌ **None** | — |
| `__import__` builtin | ❌ **None** | — |
| `importlib` module | ❌ **None** | Native stub exists |
| `sys.path` manipulation | ❌ **None** | — |
| Re-import (module caching) | ❌ **None** | — |
| `from module import *` | ❌ **None** | — |
| Submodule imports (`os.path`) | ⚠️ Smoke | test_os.py uses `os.path` |

### 2.9 Edge Cases

| Case | Covered? | Notes |
|------|----------|-------|
| Empty list `[]`, empty dict `{}`, empty tuple `()`, empty set `set()` | ❌ **None** | — |
| Zero `0`, negative int, large int (BigInt) | ❌ **None** | — |
| `float` inf, nan, negative zero | ❌ **None** | — |
| Unicode strings (multi-byte, emoji, RTL) | ❌ **None** | — |
| Escape sequences in strings (`\n`, `\t`, `\x`, `\u`) | ❌ **None** | — |
| Recursion depth limit | ❌ **None** | — |
| Very long strings / lists | ❌ **None** | — |
| Nested comprehensions | ❌ **None** | — |
| Generator expressions | ✅ Basic | test_basic.py |
| `break` / `continue` in nested loops | ❌ **None** | — |
| `else` clause on `for`/`while` | ✅ | test_completo.py |
| `del` statement (name, subscript, attr) | ❌ **None** | VM supports all three |
| Chained assignment (`a = b = c`) | ❌ **None** | — |
| Star unpacking in assignments (`a, *b, c = seq`) | ❌ **None** | UNPACK_EX in VM |
| `match`/`case` with `|`, guards, sequences | ❌ **None** | — |
| `not` operator precedence | ❌ **None** | — |
| Boolean short-circuit (`and`/`or`) | ❌ **None** | — |

---

## Section 3: Proposed Test Files (Prioritized)

### Legend

- **P0** = Must have — core feature required for ecosystem compatibility
- **P1** = Should have — important for correctness but blocked by VM gaps or lower priority
- **P2** = Nice to have — completeness / edge cases
- Complexity: **simple** (< 30 lines), **medium** (30–80 lines), **complex** (80+ lines)

---

### P0 — Core Types & Builtins

#### `tests/test_types.py` — P0 — medium

**Features**: int, float, bool, str, bytes, None operations.

```python
# Integer operations
assert 5 + 3 == 8
assert 10 - 7 == 3
assert 4 * 3 == 12
assert 15 / 4 == 3.75
assert 15 // 4 == 3
assert 15 % 4 == 3
assert 2 ** 10 == 1024
assert -5 == 0 - 5
assert +5 == 5
assert abs(-5) == 5
assert divmod(13, 4) == (3, 1)

# Bitwise
assert 5 | 3 == 7
assert 5 & 3 == 1
assert 5 ^ 3 == 6
assert ~5 == -6
assert 5 << 1 == 10
assert 5 >> 1 == 2

# Bool
assert bool(1) is True
assert bool(0) is False
assert bool([]) is False
assert bool("") is False
assert bool("a") is True

# Float
f = 3.14
assert isinstance(f, float)
assert float("inf") > 1e308
assert float("-inf") < -1e308

# String
s = "hello world"
assert s.startswith("hello")
assert s.endswith("world")
assert " ".join(["a", "b"]) == "a b"
assert "a,b,c".split(",") == ["a", "b", "c"]
assert "  hello  ".strip() == "hello"
assert "abc".upper() == "ABC"
assert "ABC".lower() == "abc"
assert "hello".find("l") == 2
assert "hello".index("l") == 2
assert "hello".count("l") == 2
assert "hello".replace("l", "x") == "hexxo"

# Bytes
b = b"hello"
assert isinstance(b, bytes)
assert b.hex() == "68656c6c6f"
assert b.decode() == "hello"
assert bytes.fromhex("68656c6c6f") == b"hello"
```

#### `tests/test_collections.py` — P0 — medium

**Features**: list, dict, tuple, set comprehensive operations.

```python
# List
lst = [3, 1, 2]
lst.sort()
assert lst == [1, 2, 3]
lst.append(4)
assert lst == [1, 2, 3, 4]
lst.insert(0, 0)
assert lst == [0, 1, 2, 3, 4]
assert lst.pop() == 4
assert lst.pop(0) == 0
assert lst == [1, 2, 3]
assert lst.count(2) == 1
assert lst.index(2) == 1
lst.extend([4, 5])
assert lst == [1, 2, 3, 4, 5]
lst.reverse()
assert lst == [5, 4, 3, 2, 1]
lst.clear()
assert lst == []

# Dict
d = {"a": 1, "b": 2}
assert d.keys() == {"a", "b"}
assert list(d.values()) == [1, 2] or sorted(d.values()) == [1, 2]
assert list(d.items()) == [("a", 1), ("b", 2)]
d.update({"c": 3})
assert d["c"] == 3
assert d.get("x", 99) == 99
assert d.pop("a") == 1
assert "a" not in d
d.setdefault("d", 4)
assert d["d"] == 4
d.clear()
assert d == {}

# Tuple
t = (1, 2, 3)
assert t.count(2) == 1
assert t.index(3) == 2
assert (1,) == (1,)
assert () == ()

# Set
s = {1, 2, 3}
s.add(4)
assert s == {1, 2, 3, 4}
s.remove(2)
assert s == {1, 3, 4}
assert {1, 2} | {2, 3} == {1, 2, 3}
assert {1, 2} & {2, 3} == {2}
assert {1, 2} - {2} == {1}
assert {1, 2} ^ {2, 3} == {1, 3}
```

#### `tests/test_builtins.py` — P0 — complex

**Features**: All built-in functions.

```python
# Numeric
assert abs(-5) == 5
assert all([True, True])
assert not all([True, False])
assert any([False, True])
assert not any([False, False])
assert bin(5) == "0b101"
assert oct(8) == "0o10"
assert hex(255) == "0xff"
assert callable(lambda: None)
assert not callable(42)
assert chr(65) == "A"
assert ord("A") == 65
assert divmod(13, 4) == (3, 1)
assert float("3.14") == 3.14
assert hash("hello") is not None
assert id(42) is not None
assert isinstance(42, int)
assert issubclass(bool, int)
assert len([1, 2, 3]) == 3
assert list("abc") == ["a", "b", "c"]
assert max(3, 7, 5) == 7
assert min(3, 7, 5) == 3
assert pow(2, 10) == 1024
assert pow(2, 10, 7) == 2  # modular exponentiation
assert range(5) is not None
assert list(range(5)) == [0, 1, 2, 3, 4]
assert repr(42) == "42"
assert str(42) == "42"
assert sum([1, 2, 3]) == 6
assert type(42) is int
assert type("hello") is str
assert zip([1, 2], ["a", "b"]) is not None
assert list(zip([1, 2], ["a", "b"])) == [(1, "a"), (2, "b")]

# map/filter
assert list(map(str, [1, 2, 3])) == ["1", "2", "3"]
assert list(filter(lambda x: x > 1, [1, 2, 3])) == [2, 3]

# sorted/reversed
assert sorted([3, 1, 2]) == [1, 2, 3]
assert list(reversed([1, 2, 3])) == [3, 2, 1]

# enumerate
assert list(enumerate(["a", "b"])) == [(0, "a"), (1, "b")]
assert list(enumerate(["a", "b"], start=1)) == [(1, "a"), (2, "b")]

# round
assert round(3.5) in (3, 4)  # banker's rounding
assert round(3.7) == 4
```

---

### P0 — Error Handling

#### `tests/test_errors.py` — P0 — medium

**Features**: try/except/else/finally, raise, assert, exception chaining.

```python
# Basic try/except
try:
    raise ValueError("test")
except ValueError:
    pass

# Multiple except handlers
try:
    raise TypeError("type error")
except ValueError:
    assert False
except TypeError:
    pass

# except as e
try:
    raise ValueError("msg")
except ValueError as e:
    assert str(e) == "msg"

# try/except/else
flag = False
try:
    pass
except ValueError:
    pass
else:
    flag = True
assert flag

# try/except/finally
flag = False
try:
    raise ValueError("test")
except ValueError:
    pass
finally:
    flag = True
assert flag

# try/finally (no except)
flag = False
try:
    pass
finally:
    flag = True
assert flag

# raise from
try:
    raise ValueError("inner")
except ValueError as e:
    try:
        raise RuntimeError("outer") from e
    except RuntimeError as e2:
        assert e2.__cause__ is e

# assert
assert True
try:
    assert False, "assertion message"
except AssertionError as e:
    assert str(e) == "assertion message"

# re-raise
try:
    try:
        raise ValueError("test")
    except ValueError:
        raise
except ValueError:
    pass
```

---

### P0 — Classes & OOP

#### `tests/test_classes.py` — P0 — complex

**Features**: Inheritance, `super()`, dunder methods, `@property`, `@staticmethod`, `@classmethod`, `__getattr__`, `__setattr__`, `__getattribute__`.

```python
# Basic inheritance
class Animal:
    def speak(self):
        return "?"
class Dog(Animal):
    def speak(self):
        return "woof"
assert Dog().speak() == "woof"

# super()
class A:
    def method(self):
        return "A"
class B(A):
    def method(self):
        return super().method() + "B"
assert B().method() == "AB"

# __init__
class Point:
    def __init__(self, x, y):
        self.x = x
        self.y = y
p = Point(3, 4)
assert p.x == 3
assert p.y == 4

# __str__ and __repr__
class MyObj:
    def __str__(self):
        return "str"
    def __repr__(self):
        return "repr"
assert str(MyObj()) == "str"
assert repr(MyObj()) == "repr" if hasattr(MyObj(), "__repr__") else True

# @property
class Circle:
    def __init__(self, r):
        self._r = r
    @property
    def area(self):
        return 3.14 * self._r * self._r
    @area.setter
    def area(self, v):
        self._r = (v / 3.14) ** 0.5
c = Circle(1)
assert c.area == 3.14

# @staticmethod
class Util:
    @staticmethod
    def add(a, b):
        return a + b
assert Util.add(2, 3) == 5

# @classmethod
class Cls:
    @classmethod
    def create(cls):
        return cls()
assert isinstance(Cls.create(), Cls)

# __getattr__
class Fallback:
    def __getattr__(self, name):
        return f"got {name}"
obj = Fallback()
assert obj.foo == "got foo"

# __setattr__
class Watch:
    def __setattr__(self, name, value):
        object.__setattr__(self, name, value.upper())
w = Watch()
w.name = "hello"
assert w.name == "HELLO"

# __call__
class Callable:
    def __call__(self, x):
        return x * 2
f = Callable()
assert f(5) == 10

# Multiple inheritance
class A:
    def method(self): return "A"
class B:
    def method(self): return "B"
class C(A, B):
    pass
assert C().method() == "A"  # MRO: C -> A -> B

# __slots__
class Slotted:
    __slots__ = ("x", "y")
    def __init__(self):
        self.x = 1
        self.y = 2
s = Slotted()
assert s.x == 1
assert s.y == 2
```

---

### P0 — Stdlib Core

#### `tests/test_json.py` — P0 — simple

**Features**: json.dumps/loads with basic types.

```python
import json
assert json.dumps({"a": 1}) == '{"a": 1}'
assert json.loads('{"a": 1}') == {"a": 1}
assert json.dumps([1, 2, 3]) == "[1, 2, 3]"
assert json.loads("[1, 2, 3]") == [1, 2, 3]
assert json.dumps("hello") == '"hello"'
assert json.loads('"hello"') == "hello"
assert json.dumps(True) == "true"
assert json.loads("true") is True
assert json.dumps(None) == "null"
```

#### `tests/test_math.py` — P0 — simple

**Features**: math module functions.

```python
import math
assert math.pi > 3.14
assert abs(math.sqrt(4) - 2.0) < 1e-10
assert abs(math.sin(0)) < 1e-10
assert abs(math.cos(0) - 1.0) < 1e-10
assert math.floor(3.7) == 3
assert math.ceil(3.2) == 4
assert abs(math.fabs(-3.5) - 3.5) < 1e-10
assert math.factorial(5) == 120
assert math.gcd(12, 8) == 4
assert math.isclose(1.0, 1.0000001)
assert math.isfinite(1.0)
assert not math.isnan(1.0)
assert math.trunc(3.7) == 3
assert math.exp(0) == 1.0
assert math.log(math.e) == 1.0
assert math.log10(100) == 2.0
assert math.pow(2, 3) == 8.0
```

#### `tests/test_itertools.py` — P0 — medium

**Features**: itertools functions.

```python
import itertools
assert list(itertools.chain([1, 2], [3, 4])) == [1, 2, 3, 4]
assert list(itertools.islice(range(10), 5)) == [0, 1, 2, 3, 4]
assert list(itertools.repeat(42, 3)) == [42, 42, 42]
assert list(itertools.count(0, 10))[:5] == [0, 10, 20, 30, 40]
assert list(itertools.cycle([1, 2, 3]))[:6] == [1, 2, 3, 1, 2, 3]

# compress
assert list(itertools.compress("ABCDEF", [1, 0, 1, 0, 1, 1])) == ["A", "C", "E", "F"]

# product
assert list(itertools.product("AB", "12")) == [("A", "1"), ("A", "2"), ("B", "1"), ("B", "2")]

# permutations
assert list(itertools.permutations("ABC", 2)) == [("A", "B"), ("A", "C"), ("B", "A"), ("B", "C"), ("C", "A"), ("C", "B")]

# combinations
assert list(itertools.combinations("ABC", 2)) == [("A", "B"), ("A", "C"), ("B", "C")]

# groupby
groups = []
for k, g in itertools.groupby("AAABBBCC"):
    groups.append((k, list(g)))
assert groups == [("A", ["A", "A", "A"]), ("B", ["B", "B", "B"]), ("C", ["C", "C"])]

# accumulate
assert list(itertools.accumulate([1, 2, 3, 4])) == [1, 3, 6, 10]
```

#### `tests/test_collections_module.py` — P0 — medium

**Features**: collections.deque, Counter, defaultdict, namedtuple.

```python
import collections

# deque
d = collections.deque([1, 2, 3])
d.append(4)
d.appendleft(0)
assert list(d) == [0, 1, 2, 3, 4]
assert d.pop() == 4
assert d.popleft() == 0

# Counter
c = collections.Counter("aabbccc")
assert c["a"] == 2
assert c["c"] == 3
assert c.most_common(2) == [("c", 3), ("a", 2)]

# defaultdict
dd = collections.defaultdict(int)
dd["a"] += 1
assert dd["a"] == 1
assert dd["b"] == 0  # default

# namedtuple
Point = collections.namedtuple("Point", ["x", "y"])
p = Point(3, 4)
assert p.x == 3
assert p.y == 4
assert p[0] == 3
assert p[1] == 4
```

---

### P1 — Functions & Generators

#### `tests/test_functions_advanced.py` — P1 — complex

**Features**: Decorator chaining, functools.lru_cache, keyword-only args, positional-only args, recursion, closures with mutation.

```python
# Decorator chaining
def decorate_A(f):
    def wrapper(*args, **kwargs):
        return "<A>" + f(*args, **kwargs) + "</A>"
    return wrapper

def decorate_B(f):
    def wrapper(*args, **kwargs):
        return "<B>" + f(*args, **kwargs) + "</B>"
    return wrapper

@decorate_A
@decorate_B
def greet(name):
    return f"Hello {name}"

result = greet("World")
assert result == "<A><B>Hello World</B></A>"

# Closures with mutation
def make_counter():
    count = [0]
    def inc():
        count[0] += 1
        return count[0]
    return inc

c1 = make_counter()
c2 = make_counter()
assert c1() == 1
assert c1() == 2
assert c2() == 1
assert c1() == 3

# Recursion
def factorial(n):
    return 1 if n <= 1 else n * factorial(n - 1)
assert factorial(5) == 120
assert factorial(10) == 3628800

# Keyword-only args
def kw_only(*, a, b=10):
    return a + b
assert kw_only(a=1) == 11
assert kw_only(a=1, b=2) == 3

# Default args are evaluated once
defaults = []
def append_default(x, lst=[]):
    lst.append(x)
    return lst
assert append_default(1) == [1]
assert append_default(2) == [1, 2]  # CPython behavior

# yield from
def inner():
    yield 1
    yield 2
def outer():
    yield from inner()
    yield 3
assert list(outer()) == [1, 2, 3]

# Generator .send() .throw() .close()
def gen_send():
    val = yield 1
    yield val
g = gen_send()
assert next(g) == 1
assert g.send(42) == 42
```

#### `tests/test_generators.py` — P1 — medium

**Features**: Multiple yield, generator expressions, send/throw/close, yield from.

```python
# Multi-yield generator (KNOWN BLOCKER: StopIteration after first next())
def multi_yield():
    yield 1
    yield 2
    yield 3

g = multi_yield()
assert next(g) == 1
assert next(g) == 2
assert next(g) == 3
try:
    next(g)
    assert False, "Expected StopIteration"
except StopIteration:
    pass

# Generator expression
g = (x * x for x in range(5))
assert list(g) == [0, 1, 4, 9, 16]

# Generator with send
def echo():
    v = yield
    yield v
g = echo()
next(g)  # prime
result = g.send(42)
assert result == 42

# Generator throw
def will_raise():
    try:
        yield 1
    except ValueError:
        yield "caught"
g = will_raise()
assert next(g) == 1
assert g.throw(ValueError) == "caught"
```

---

### P1 — Stdlib Modules

#### `tests/test_io.py` — P1 — simple

**Features**: io.StringIO.

```python
import io
buf = io.StringIO()
buf.write("hello ")
buf.write("world")
assert buf.getvalue() == "hello world"

buf2 = io.StringIO("initial content")
assert buf2.read() == "initial content"
buf2.seek(0)
assert buf2.read(4) == "init"

# readline
buf3 = io.StringIO("line1\nline2\nline3")
assert buf3.readline() == "line1\n"
assert buf3.readline() == "line2\n"
```

#### `tests/test_sys.py` — P1 — simple

**Features**: sys module attributes and functions.

```python
import sys
assert hasattr(sys, "path")
assert isinstance(sys.path, list)
assert hasattr(sys, "version")
assert hasattr(sys, "version_info")
assert hasattr(sys, "hexversion")
assert hasattr(sys, "argv")
assert hasattr(sys, "modules")
assert hasattr(sys, "platform")
assert hasattr(sys, "stdout")

# stderr/stdin exist
assert sys.stdout is not None
assert sys.stderr is not None
```

#### `tests/test_random.py` — P1 — medium

**Features**: random module.

```python
import random
# Basic
assert 0 <= random.random() < 1
assert isinstance(random.randint(1, 10), int)
assert 1 <= random.randint(1, 10) <= 10

# Choices
items = ["a", "b", "c"]
choice = random.choice(items)
assert choice in items

# Shuffle
lst = [1, 2, 3, 4, 5]
random.shuffle(lst)
assert sorted(lst) == [1, 2, 3, 4, 5]  # same elements

# Sample
sample = random.sample([1, 2, 3, 4, 5], 3)
assert len(sample) == 3
assert all(x in [1, 2, 3, 4, 5] for x in sample)

# Reproducibility
random.seed(42)
a = random.random()
random.seed(42)
b = random.random()
assert a == b
```

#### `tests/test_datetime.py` — P1 — medium

**Features**: datetime module.

```python
import datetime
today = datetime.date.today()
assert isinstance(today, datetime.date)

dt = datetime.datetime(2024, 1, 15, 10, 30, 0)
assert dt.year == 2024
assert dt.month == 1
assert dt.day == 15
assert dt.hour == 10
assert dt.minute == 30
assert dt.second == 0

d = datetime.date(2024, 1, 15)
assert d.year == 2024
assert d.month == 1
assert d.day == 15

t = datetime.time(10, 30, 0)
assert t.hour == 10
assert t.minute == 30

td = datetime.timedelta(days=1, hours=2)
assert td.days == 1
assert td.seconds == 7200
assert td.total_seconds() == 93600

# date arithmetic
d2 = d + td
assert d2.day == 16
```

#### `tests/test_hashlib.py` — P1 — simple

**Features**: hashlib module.

```python
import hashlib
md5 = hashlib.md5(b"hello")
assert md5.hexdigest() == "5d41402abc4b2a76b9719d911017c592"
sha1 = hashlib.sha1(b"hello")
assert sha1.hexdigest() == "aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d"
sha256 = hashlib.sha256(b"hello")
assert len(sha256.hexdigest()) == 64
```

#### `tests/test_copy.py` — P1 — simple

**Features**: copy.copy, copy.deepcopy.

```python
import copy

# shallow copy
lst = [1, [2, 3], 4]
lst2 = copy.copy(lst)
assert lst == lst2
assert lst[1] is lst2[1]  # same inner list

# deep copy
lst3 = copy.deepcopy(lst)
assert lst == lst3
assert lst[1] is not lst3[1]  # different inner list

# custom __copy__
class HasCopy:
    def __init__(self, v):
        self.v = v
    def __copy__(self):
        return HasCopy(self.v + 1)
obj = HasCopy(5)
obj2 = copy.copy(obj)
assert obj2.v == 6
```

#### `tests/test_enum.py` — P1 — medium

**Features**: enum module.

```python
from enum import Enum, auto

class Color(Enum):
    RED = 1
    GREEN = 2
    BLUE = 3

assert Color.RED.value == 1
assert Color(1) == Color.RED
assert Color.RED.name == "RED"
assert list(Color) == [Color.RED, Color.GREEN, Color.BLUE]

class AutoEnum(Enum):
    A = auto()
    B = auto()
assert AutoEnum.A.value == 1
assert AutoEnum.B.value == 2
```

#### `tests/test_operator.py` — P1 — simple

**Features**: operator module functions.

```python
import operator
assert operator.add(2, 3) == 5
assert operator.sub(10, 3) == 7
assert operator.mul(4, 5) == 20
assert operator.truediv(10, 3) > 3.33
assert operator.floordiv(10, 3) == 3
assert operator.mod(10, 3) == 1
assert operator.pow(2, 3) == 8
assert operator.lt(1, 2) is True
assert operator.le(1, 1) is True
assert operator.eq(1, 1) is True
assert operator.ne(1, 2) is True
assert operator.gt(2, 1) is True
assert operator.ge(2, 2) is True
assert operator.truth(1) is True
assert operator.truth(0) is False
assert operator.not_(True) is False
assert operator.contains([1, 2, 3], 2) is True
assert operator.countOf([1, 2, 2, 3], 2) == 2
assert operator.indexOf([1, 2, 3], 2) == 1
assert operator.getitem([1, 2, 3], 1) == 2
assert operator.length_hint([1, 2, 3]) == 3
```

---

### P1 — Import System

#### `tests/test_imports.py` — P1 — complex

**Features**: Absolute imports, relative imports, package __init__, circular imports, from ... import *, import error handling.

```python
# Create test package structure first
import os, sys
test_dir = "/tmp/test_imports_" + str(os.getpid())
os.makedirs(test_dir, exist_ok=True)

# Write __init__.py
with open(os.path.join(test_dir, "__init__.py"), "w") as f:
    f.write("__version__ = '1.0'\n")

# Write submodule
with open(os.path.join(test_dir, "sub.py"), "w") as f:
    f.write("Z = 42\n")

sys.path.insert(0, "/tmp")

# Absolute import
import import_test  # or whatever the package is named
assert import_test.__version__ == "1.0"

# From import
from import_test.sub import Z
assert Z == 42

# Relative import (within package)
# ... would require structured package

# Cleanup
# import shutil; shutil.rmtree(test_dir)
```

---

### P1 — Edge Cases

#### `tests/test_edge_cases.py` — P1 — complex

**Features**: Empty collections, large numbers, recursion depth, unicode, escape sequences, chained assignment, `del`, star unpacking in assignment, comparison chaining.

```python
# Empty collections
assert [] == []
assert {} == {}
assert set() == set()
assert () == ()
assert list([]) == []
assert dict({}) == {}
assert tuple(()) == ()

# Large numbers
big = 10 ** 100
assert big > 0
assert big + 1 > big
assert big * 2 == 2 * big
assert big // 10 == 10 ** 99

# Unicode
assert "héllo" == "héllo"
assert "héllo".upper() == "HÉLLO"
emoji = "🔥"
assert len(emoji) == 1
assert emoji == emoji

# Escape sequences
assert "\n" != "n"
assert "\t" == "\t"
assert "\\" == "\\"
assert "\x41" == "A"
assert "\u00e9" == "é"

# Chained assignment
a = b = c = 42
assert a == 42
assert b == 42
assert c == 42

# del statement
x = 42
del x
try:
    print(x)
    assert False
except NameError:
    pass

# del subscript
lst = [1, 2, 3, 4]
del lst[1]
assert lst == [1, 3, 4]

# Comparison chaining
assert 1 < 2 < 3
assert not (1 < 2 > 3)
assert 1 < 2 < 3 < 4 < 5

# Boolean short-circuit
def side_effect(lst, val):
    lst.append(val)
    return val

seen = []
result = False and side_effect(seen, 1)
assert result is False
assert 1 not in seen  # side_effect NOT called

result = True or side_effect(seen, 2)
assert result is True
assert 2 not in seen  # side_effect NOT called

# String formatting
assert f"{42}" == "42"
assert f"{42:06d}" == "000042"
assert f"{3.14159:.2f}" == "3.14"

# Slice operations
lst = list(range(10))
assert lst[::2] == [0, 2, 4, 6, 8]
assert lst[::-1] == [9, 8, 7, 6, 5, 4, 3, 2, 1, 0]
assert lst[2:8:2] == [2, 4, 6]
assert lst[5:2:-1] == [5, 4, 3]
```

---

### P2 — Complex Flows (Integration Tests)

#### `tests/test_context_managers.py` — P2 — medium

**Features**: with statement, custom context managers, __enter__/__exit__.

```python
# Basic with
class MyCM:
    def __enter__(self):
        return 42
    def __exit__(self, *args):
        pass

with MyCM() as x:
    assert x == 42

# Exception handling in __exit__
class Suppressor:
    def __exit__(self, exc_type, exc_val, exc_tb):
        return isinstance(exc_val, ValueError)

with Suppressor():
    raise ValueError("suppressed")

# Nested with
class Tracker:
    entries = []
    def __enter__(self):
        self.entries.append("enter")
        return self
    def __exit__(self, *args):
        self.entries.append("exit")

with Tracker() as t:
    pass
assert t.entries == ["enter", "exit"]
```

#### `tests/test_comprehensions.py` — P2 — medium

**Features**: Nested list/dict/set comprehensions, with if clauses.

```python
# Nested list comp
matrix = [[1, 2], [3, 4], [5, 6]]
flat = [x for row in matrix for x in row]
assert flat == [1, 2, 3, 4, 5, 6]

# With if
evens = [x for x in range(10) if x % 2 == 0]
assert evens == [0, 2, 4, 6, 8]

# Dict comp
squares = {x: x*x for x in range(5)}
assert squares == {0: 0, 1: 1, 2: 4, 3: 9, 4: 4}

# Set comp
unique = {x % 3 for x in range(10)}
assert unique == {0, 1, 2}

# Filtered dict comp
even_sq = {x: x*x for x in range(10) if x % 2 == 0}
assert even_sq == {0: 0, 2: 4, 4: 16, 6: 36, 8: 64}
```

#### `tests/test_dataclasses.py` — P2 — medium

**Features**: dataclasses module.

```python
from dataclasses import dataclass, field, asdict

@dataclass
class Person:
    name: str
    age: int = 0

p = Person("Alice", 30)
assert p.name == "Alice"
assert p.age == 30
assert repr(p) != ""

# asdict
d = asdict(p)
assert d == {"name": "Alice", "age": 30}

# field with default_factory
@dataclass
class Config:
    tags: list = field(default_factory=list)
    values: dict = field(default_factory=dict)

c = Config()
assert c.tags == []
assert c.values == {}
```

#### `tests/test_typing.py` — P2 — simple

**Features**: typing module stubs (basic type hints, List, Dict, Optional, Union).

```python
from typing import List, Dict, Optional, Union, Any
assert List[int] is not None
assert Dict[str, int] is not None
assert Optional[str] is not None
assert Union[int, str] is not None

# Usage as isinstance (if supported)
x: List[int] = [1, 2, 3]
assert isinstance(x, list)
```

#### `tests/test_struct.py` — P2 — simple

**Features**: struct module.

```python
import struct
data = struct.pack(">i4s", 42, b"test")
unpacked = struct.unpack(">i4s", data)
assert unpacked == (42, b"test")

# Various format characters
data2 = struct.pack("<HHI", 1, 2, 3)
assert struct.unpack("<HHI", data2) == (1, 2, 3)
assert struct.calcsize(">i") == 4
assert struct.calcsize(">d") == 8
```

#### `tests/test_ast_literal.py` — P2 — simple

**Features**: ast.literal_eval.

```python
import ast
assert ast.literal_eval("42") == 42
assert ast.literal_eval("3.14") == 3.14
assert ast.literal_eval("[1, 2, 3]") == [1, 2, 3]
assert ast.literal_eval("{'a': 1}") == {"a": 1}
assert ast.literal_eval("(1, 2)") == (1, 2)
assert ast.literal_eval("True") is True
assert ast.literal_eval("None") is None
```

#### `tests/test_pprint.py` — P2 — simple

**Features**: pprint module.

```python
import pprint
import io

buf = io.StringIO()
data = {"a": [1, 2, 3], "b": {"c": "d"}}
pprint.pprint(data, stream=buf)
output = buf.getvalue()
assert "a" in output
assert "b" in output
```

#### `tests/test_bisect.py` — P2 — simple

**Features**: bisect module.

```python
import bisect
lst = [1, 3, 5, 7]
assert bisect.bisect_left(lst, 4) == 2
assert bisect.bisect_right(lst, 4) == 2
assert bisect.bisect(lst, 4) == 2

bisect.insort(lst, 4)
assert lst == [1, 3, 4, 5, 7]
```

#### `tests/test_heapq.py` — P2 — simple

**Features**: heapq module.

```python
import heapq
lst = [3, 1, 4, 1, 5, 9, 2, 6]
heapq.heapify(lst)
# After heapify, smallest element is at position 0
assert lst[0] == 1
assert heapq.heappop(lst) == 1
assert heapq.heappop(lst) == 1  # second smallest

# nlargest / nsmallest
lst2 = [5, 2, 8, 1, 9]
assert heapq.nlargest(3, lst2) == [9, 8, 5]
assert heapq.nsmallest(3, lst2) == [1, 2, 5]
```

#### `tests/test_re_advanced.py` — P2 — medium

**Features**: Advanced regex (groups, named groups, flags, sub with function).

```python
import re

# Groups
m = re.match(r"(\d+)-(\w+)", "123-abc")
assert m is not None
assert m.group(0) == "123-abc"
assert m.group(1) == "123"
assert m.group(2) == "abc"
assert m.groups() == ("123", "abc")

# Named groups
m = re.match(r"(?P<num>\d+)", "42abc")
assert m is not None
assert m.group("num") == "42"

# Flags
m = re.match(r"hello", "HELLO", re.IGNORECASE)
assert m is not None

# sub with function
result = re.sub(r"\d+", lambda m: str(int(m.group(0)) * 2), "a1b2c3")
assert result == "a2b4c6"

# split with maxsplit
result = re.split(r"\s+", "a b   c d", maxsplit=2)
assert result == ["a", "b", "c d"]

# finditer
matches = list(re.finditer(r"\d+", "a1b2c3"))
assert len(matches) == 3
assert matches[0].group() == "1"

# Subn (count)
result, count = re.subn(r"a", "x", "aaabbb")
assert result == "xxxbbb"
assert count == 3
```

#### `tests/test_functools_advanced.py` — P2 — medium

**Features**: functools.lru_cache, singledispatch, cached_property, partial with keywords.

```python
import functools

# lru_cache
@functools.lru_cache(maxsize=128)
def fib(n):
    return n if n < 2 else fib(n-1) + fib(n-2)
assert fib(10) == 55
assert fib.cache_info().hits > 0

# partial with keywords
def f(a, b, c):
    return a + b + c
p = functools.partial(f, c=3)
assert p(1, 2) == 6

# singledispatch
if hasattr(functools, "singledispatch"):
    @functools.singledispatch
    def func(arg):
        return "default"
    @func.register(int)
    def _(arg):
        return "int"
    assert func("hello") == "default"
    assert func(42) == "int"
```

#### `tests/test_os_advanced.py` — P2 — medium

**Features**: os.path functions, os.environ, os.listdir, os.remove, etc.

```python
import os

# environ
assert isinstance(os.environ, dict)
assert "PATH" in os.environ

# path functions
assert os.path.isdir("/tmp")
assert os.path.isfile("/etc/passwd") or True  # may not exist
assert os.path.exists("/tmp")
assert os.path.basename("/a/b/c") == "c"
assert os.path.dirname("/a/b/c") == "/a/b"
assert os.path.splitext("file.txt") == ("file", ".txt")
assert os.path.join("a", "b", "c") == "a/b/c"
assert os.path.normpath("/a/./b/../c") == "/a/c"

# listdir
files = os.listdir("/tmp")
assert isinstance(files, list)
```

#### `tests/test_inspect.py` — P2 — medium

**Features**: inspect module (getsource, getfile, getsourcelines, signature).

```python
import inspect

def sample_func(a, b=10):
    """docstring"""
    return a + b

# Source inspection
src = inspect.getsource(sample_func)
assert "def sample_func" in src

# Signature
sig = inspect.signature(sample_func)
assert "a" in sig.parameters

# isfunction / ismethod / isclass
assert inspect.isfunction(sample_func)
assert inspect.isclass(int)

# getdoc
doc = inspect.getdoc(sample_func)
assert doc == "docstring"
```

---

### P2 — OOP Patterns

#### `tests/test_oop_patterns.py` — P2 — complex

**Features**: Factory, singleton, adapter, observer patterns (simplified).

```python
# Factory pattern
class Factory:
    def create(self, type_name):
        if type_name == "a":
            return TypeA()
        elif type_name == "b":
            return TypeB()

class TypeA:
    def name(self): return "A"

class TypeB:
    def name(self): return "B"

f = Factory()
assert f.create("a").name() == "A"
assert f.create("b").name() == "B"

# Singleton pattern
class Singleton:
    _instance = None
    def __new__(cls):
        if cls._instance is None:
            cls._instance = super().__new__(cls)
        return cls._instance

s1 = Singleton()
s2 = Singleton()
assert s1 is s2

# Adapter pattern
class Adaptee:
    def specific_request(self):
        return "specific"

class Adapter:
    def __init__(self, adaptee):
        self.adaptee = adaptee
    def request(self):
        return self.adaptee.specific_request()

adapter = Adapter(Adaptee())
assert adapter.request() == "specific"
```

#### `tests/test_recursive_algorithms.py` — P2 — complex

**Features**: Factorial, fibonacci, tree traversal, quicksort.

```python
# Factorial
def fact(n):
    return 1 if n <= 1 else n * fact(n - 1)
assert fact(0) == 1
assert fact(5) == 120
assert fact(10) == 3628800

# Fibonacci
def fib(n):
    a, b = 0, 1
    for _ in range(n):
        a, b = b, a + b
    return a
assert fib(0) == 0
assert fib(1) == 1
assert fib(10) == 55

# Quicksort
def qsort(lst):
    if len(lst) <= 1:
        return lst
    pivot = lst[0]
    less = [x for x in lst[1:] if x <= pivot]
    greater = [x for x in lst[1:] if x > pivot]
    return qsort(less) + [pivot] + qsort(greater)

assert qsort([3, 1, 4, 1, 5, 9]) == [1, 1, 3, 4, 5, 9]

# Tree traversal
class TreeNode:
    def __init__(self, val, left=None, right=None):
        self.val = val
        self.left = left
        self.right = right

def inorder(node):
    if node is None:
        return []
    return inorder(node.left) + [node.val] + inorder(node.right)

root = TreeNode(2, TreeNode(1), TreeNode(3))
assert inorder(root) == [1, 2, 3]
```

#### `tests/test_django_import_chain.py` — P2 — complex

**Features**: Simulated complex import chain (django → utils → functional → wraps → closures).

```python
# Simulate the Django import chain
# This tests nested imports, closures through import chains, and cross-module references

import sys

# Create temp module simulating django.utils.functional
code = """
def wraps(f):
    def wrapper(*args, **kwargs):
        return f(*args, **kwargs)
    wrapper.__name__ = f.__name__
    return wrapper

class cached_property:
    def __init__(self, func):
        self.func = func
    def __get__(self, obj, cls):
        if obj is None:
            return self
        value = self.func(obj)
        obj.__dict__[self.func.__name__] = value
        return value

def lazy(func):
    result = None
    def evaluate():
        nonlocal result
        if result is None:
            result = func()
        return result
    return evaluate
"""

import tempfile
import os

tmpdir = tempfile.mkdtemp()
modpath = os.path.join(tmpdir, "django_utils_functional.py")
with open(modpath, "w") as f:
    f.write(code)

sys.path.insert(0, tmpdir)

# Import the module
import django_utils_functional as duf

# Test the wraps
def my_func():
    return 42

decorated = duf.wraps(my_func)(lambda: 42)
assert decorated() == 42

# Test cached_property
class MyClass:
    @duf.cached_property
    def expensive(self):
        return 99

obj = MyClass()
assert obj.expensive == 99

# Test lazy
call_count = [0]
def expensive():
    call_count[0] += 1
    return 42

lazy_fn = duf.lazy(expensive)
assert lazy_fn() == 42
assert lazy_fn() == 42  # cached
assert call_count[0] == 1  # only called once
```

---

## Section 3.3: Summary Table of Proposed Tests

| File | Priority | Complexity | VM Blocker? | Est. Lines |
|------|----------|------------|-------------|------------|
| `test_types.py` | P0 | medium | No | 70 |
| `test_collections.py` | P0 | medium | No | 80 |
| `test_builtins.py` | P0 | complex | No | 100 |
| `test_errors.py` | P0 | medium | No | 70 |
| `test_classes.py` | P0 | complex | No | 120 |
| `test_json.py` | P0 | simple | No | 20 |
| `test_math.py` | P0 | simple | No | 30 |
| `test_itertools.py` | P0 | medium | No | 50 |
| `test_collections_module.py` | P0 | medium | No | 50 |
| `test_functions_advanced.py` | P1 | complex | ⚠️ Blocked (yield from, multi-yield) | 80 |
| `test_generators.py` | P1 | medium | ⚠️ Blocked (multi-yield) | 50 |
| `test_io.py` | P1 | simple | No | 30 |
| `test_sys.py` | P1 | simple | No | 20 |
| `test_random.py` | P1 | medium | No | 40 |
| `test_datetime.py` | P1 | medium | No | 50 |
| `test_hashlib.py` | P1 | simple | No | 20 |
| `test_copy.py` | P1 | simple | No | 30 |
| `test_enum.py` | P1 | medium | No | 30 |
| `test_operator.py` | P1 | simple | No | 40 |
| `test_imports.py` | P1 | complex | No | 60 |
| `test_edge_cases.py` | P1 | complex | No | 80 |
| `test_context_managers.py` | P2 | medium | No | 40 |
| `test_comprehensions.py` | P2 | medium | No | 40 |
| `test_dataclasses.py` | P2 | medium | No | 30 |
| `test_typing.py` | P2 | simple | No | 15 |
| `test_struct.py` | P2 | simple | No | 20 |
| `test_ast_literal.py` | P2 | simple | No | 20 |
| `test_pprint.py` | P2 | simple | No | 20 |
| `test_bisect.py` | P2 | simple | No | 15 |
| `test_heapq.py` | P2 | simple | No | 25 |
| `test_re_advanced.py` | P2 | medium | No | 50 |
| `test_functools_advanced.py` | P2 | medium | No | 40 |
| `test_os_advanced.py` | P2 | medium | No | 35 |
| `test_inspect.py` | P2 | medium | No | 30 |
| `test_oop_patterns.py` | P2 | complex | No | 80 |
| `test_recursive_algorithms.py` | P2 | complex | No | 60 |
| `test_django_import_chain.py` | P2 | complex | ⚠️ Partial (import system) | 80 |

---

## Section 4: Python Flows Not Yet Covered

These are larger integration scenarios that exercise multiple features together:

### 4.1 Django Import Chain

```
import django → django.utils → django.utils.functional → wraps → closures → walrus → classes
```

**Key interactions**: Nested imports, module attributes, closure through import chain, decorators (`@wraps`, `@cached_property`), `nonlocal` in imported functions.

**What to test**: The `test_django_import_chain.py` above covers this.

### 4.2 JSON Encode/Decode Pipeline

```python
import json

# Encode
data = {
    "string": "hello",
    "int": 42,
    "float": 3.14,
    "list": [1, 2, 3],
    "dict": {"nested": True},
    "bool": False,
    "none": None,
}
encoded = json.dumps(data)
decoded = json.loads(encoded)
assert decoded == data

# Custom serialization
class Custom:
    def __init__(self, v):
        self.v = v

class CustomEncoder(json.JSONEncoder):
    def default(self, obj):
        if isinstance(obj, Custom):
            return {"__custom__": obj.v}
        return super().default(obj)

result = json.dumps(Custom(42), cls=CustomEncoder)
assert result == '{"__custom__": 42}'
```

**Key interactions**: Recursive traversal, dict/str conversion, type dispatch, class inheritance.

### 4.3 Deep Copy with Custom Methods

```python
import copy

class DeepCopyable:
    def __init__(self, value, children=None):
        self.value = value
        self.children = children or []
    def __copy__(self):
        return DeepCopyable(self.value, self.children)
    def __deepcopy__(self, memo):
        return DeepCopyable(
            copy.deepcopy(self.value, memo),
            [copy.deepcopy(c, memo) for c in self.children]
        )

root = DeepCopyable(1, [DeepCopyable(2), DeepCopyable(3)])
clone = copy.deepcopy(root)
assert clone.value == root.value
assert clone is not root
assert clone.children[0] is not root.children[0]
```

**Key interactions**: Recursion, memo dict, `__copy__`/`__deepcopy__` protocol chaining, is vs equality.

### 4.4 Decorator Chaining

```python
def bold(f):
    def wrapper(*args, **kwargs):
        return f"<b>{f(*args, **kwargs)}</b>"
    return wrapper

def italic(f):
    def wrapper(*args, **kwargs):
        return f"<i>{f(*args, **kwargs)}</i>"
    return wrapper

@bold
@italic
def greet(name):
    return f"Hello {name}"

assert greet("World") == "<b><i>Hello World</i></b>"
```

**Key interactions**: Nested function definitions, `*args`/`**kwargs` forwarding through multiple layers, closure capture of `f`.

### 4.5 Context Manager Stacking

```python
class Indent:
    level = 0
    def __enter__(self):
        Indent.level += 1
        return self
    def __exit__(self, *args):
        Indent.level -= 1

with Indent():
    assert Indent.level == 1
    with Indent():
        assert Indent.level == 2
    assert Indent.level == 1
assert Indent.level == 0
```

**Key interactions**: `with` statement nesting, `__enter__`/`__exit__` protocol, class-level state.

### 4.6 Recursive Tree Traversal

```python
class Node:
    def __init__(self, value, left=None, right=None):
        self.value = value
        self.left = left
        self.right = right

def dfs_inorder(node, result=None):
    if result is None:
        result = []
    if node:
        dfs_inorder(node.left, result)
        result.append(node.value)
        dfs_inorder(node.right, result)
    return result

def dfs_preorder(node, result=None):
    if result is None:
        result = []
    if node:
        result.append(node.value)
        dfs_preorder(node.left, result)
        dfs_preorder(node.right, result)
    return result

root = Node(1, Node(2, Node(4), Node(5)), Node(3, Node(6)))
assert dfs_inorder(root) == [4, 2, 5, 1, 6, 3]
assert dfs_preorder(root) == [1, 2, 4, 5, 3, 6]
```

**Key interactions**: Recursion depth, mutable default args (but handled via sentinel), object attribute access, class instantiation.

### 4.7 Serialization/Deserialization Round-trip

```python
class Serializable:
    def __init__(self, **kwargs):
        self.__dict__.update(kwargs)
    def to_dict(self):
        return self.__dict__
    @classmethod
    def from_dict(cls, data):
        return cls(**data)

obj = Serializable(name="test", count=42, active=True)
data = obj.to_dict()
clone = Serializable.from_dict(data)
assert clone.name == "test"
assert clone.count == 42
assert clone.active is True
```

**Key interactions**: `__dict__`, `**kwargs` unpacking, `@classmethod`, dict iteration.

---

## Section 5: Known VM Gaps (Blockers)

### 5.1 Missing VM Opcode Handlers

These opcodes are **defined** in `bytecode.rs` but have **no handler** in `vm.rs`:

| Opcode | Def. in | Needed For | Impact | Workaround |
|--------|---------|------------|--------|------------|
| `CALL_FUNCTION_EX` | bytecode.rs:13 | `func(*args, **kwargs)` call syntax | `func(*lst)`, `func(**dct)` fail at runtime | Compiler uses `CALL` for non-starred calls |
| `CALL_KW` | bytecode.rs:14 | Keyword argument calls with more args | May crash on some complex calls | `CALL` opcode handles basic case |
| `BUILD_TUPLE_UNPACK` | ❌ Not defined | `*args` in tuple expressions `(1, *t)` | `x = [1, *lst]` in calls | Skip in test |
| `BUILD_MAP_UNPACK` | ❌ Not defined | `**kwargs` in dict expressions `{**d}` | `{**d1, **d2}` fails | Dict merge `DICT_MERGE` exists for `{**d}` |
| `CALL_INTRINSIC_1` | bytecode.rs:97 | PEP 523 intrinsic operations | Unused in normal code (opt-in only) | None needed |
| `CALL_INTRINSIC_2` | bytecode.rs:98 | Intrinsic operations | Unused in normal code (opt-in only) | None needed |
| `GET_LEN` | bytecode.rs:85 | Optimized `len()` | Fallback to `BINARY_OP 13` works | ✅ Works via fallback |
| `MATCH_MAPPING` | bytecode.rs:86 | Pattern matching: mapping check | `match d: case {}:` pattern matching | Match works with simple values |
| `MATCH_SEQUENCE` | bytecode.rs:87 | Pattern matching: sequence check | `match lst: case [a, b]:` | Match works with simple values |
| `MATCH_KEYS` | bytecode.rs:88 | Pattern matching: key lookup | `match d: case {"k": v}:` | Match works with simple values |

### 5.2 Compiler Gaps (No Code Generation)

These features are parsed but the compiler doesn't produce correct bytecode:

| Feature | Compiler Issue | Impact |
|---------|---------------|--------|
| `func(*args)` in calls | Compiler doesn't emit `BUILD_TUPLE_UNPACK` or `CALL_FUNCTION_EX` — just passes `*args` as a normal arg | `star_func(1, *args)` will pass `*args` as a tuple, not unpacked elements |
| `func(**kwargs)` in calls | Compiler doesn't emit `BUILD_MAP_UNPACK` — just passes `**kwargs` as a normal kwarg | `func(**dct)` may fail or pass incorrect args |
| `star in tuples` `(1, *t)` | Doesn't emit `BUILD_TUPLE_UNPACK` | `(1,) + tuple(t)` works as workaround |
| `{**d1, **d2}` dict merge | `DICT_MERGE` exists in VM but compiler may not emit correctly | `{**d}` basic form likely works |
| `a, *b, c = seq` star unpack | `UNPACK_EX` exists in VM but compiler may not emit for assignments | `a, b, c = seq[0], seq[1:-1], seq[-1]` workaround |

### 5.3 Generator / Coroutine Bugs

| Bug | Location | Impact | Status |
|-----|----------|--------|--------|
| Generators stop after first `next()` on multi-yield | VM `YIELD_VALUE` / `FOR_ITER` interaction | `list(generator_with_two_yields)` fails | ⚠️ **Known bug** |
| `async def / await` no event loop to run | asyncio module stub-only | Async code compiles but can't execute | ⚠️ **Blocked** |
| `yield from` compiler gap | compiler.rs doesn't handle `Expr::YieldFrom` | `yield from g` not supported | ❌ **Missing** |

### 5.4 Object System Gaps

| Feature | Issue | Impact |
|---------|-------|--------|
| `__dict__` on instances | `obj.__dict__` may return a dict-like object, but attribute vs dict-item sync may be wrong | `vars(obj)` or `obj.__dict__` may give unexpected results |
| `super(Type, self)` with arguments | Compiler or VM may not handle two-arg `super()` | Advanced super() patterns may fail |
| `__slots__` | `extract_slots` exists in object.rs but not fully wired | Cannot prevent arbitrary attribute setting |
| `__class_getitem__` (PEP 560) | Not implemented | `List[int]` may fail on generic types |
| `__init_subclass__` (PEP 487) | Not implemented | Class inheritance hooks missing |
| `__set_name__` (PEP 487) | Not implemented | Descriptor name not auto-assigned |

### 5.5 Module Gaps

| Module | Gap | Impact |
|--------|-----|--------|
| `asyncio` | No event loop; coroutine type exists but idle | `async def` code cannot execute at runtime |
| `dataclasses` | Minimal/partial at best | `@dataclass` decorator may not work fully |
| `typing` | Stubs only; `List[int]` etc. may fail | `typing` import works but generics may not |
| `unittest` | Stub only (TestCase name only) | Cannot run real tests with unittest |
| `importlib` | Stub only | Dynamic imports not supported |
| `pickle` | Minimal stub | Object serialization not functional |
| `concurrent.futures` | ❌ Not implemented | Thread/process pool not available |
| `multiprocessing` | ❌ Not implemented | Process spawning not available |

### 5.6 Quick Check: Can You Test This?

Before writing a test, check if it will actually execute:

| Pattern | Testable? | Why |
|---------|-----------|-----|
| `f"hello {name}"` | ✅ Yes | f-strings work |
| `f"{x!r}"` | ✅ Yes | repr conversion works |
| `f"{x:>10}"` | ✅ Yes | format spec works |
| `func(*args)` | ❌ No | `CALL_FUNCTION_EX` not implemented |
| `func(**kwargs)` | ❌ No | `BUILD_MAP_UNPACK` not implemented |
| `yield 1; yield 2` | ❌ No | Multi-yield bug |
| `async def f(): ...` | ⚠️ Partial | Parses but no event loop to run |
| `match x: case [a, b]:` | ⚠️ Partial | Match works for simple value patterns |
| `super().__init__()` | ⚠️ Partial | No-arg super() may work |
| `@dataclass` | ❌ No | Stub only |
| `from typing import List` | ⚠️ Partial | Module imports but generics may not work |
| `import json; json.dumps(x)` | ✅ Yes | Native implementation |
| `import math; math.sqrt(4)` | ✅ Yes | Native implementation |
| `with open("f"): pass` | ❌ No | `open()` builtin not implemented |

---

## Section 6: Recommended Implementation Order

### Phase 1 — Immediate (Fill coverage gaps, no VM changes needed)

These tests should be written FIRST — they test features that already work:

1. `test_types.py` — int, float, bool, str, bytes, bitwise operators
2. `test_collections.py` — list, dict, tuple, set comprehensive
3. `test_builtins.py` — all built-in functions
4. `test_errors.py` — try/except/else/finally, raise, assert, chaining
5. `test_classes.py` — inheritance, super(), dunders, @property, @staticmethod, @classmethod
6. `test_json.py` — json.dumps/loads
7. `test_math.py` — math module
8. `test_itertools.py` — itertools module
9. `test_collections_module.py` — deque, Counter, defaultdict, namedtuple

### Phase 2 — Stdlib Expansion (Test existing native modules)

10. `test_io.py` — io.StringIO
11. `test_sys.py` — sys attributes
12. `test_random.py` — random module
13. `test_datetime.py` — datetime module
14. `test_hashlib.py` — hashlib module
15. `test_copy.py` — copy.copy/deepcopy
16. `test_enum.py` — enum module
17. `test_operator.py` — operator module

### Phase 3 — Edge Cases & Integration

18. `test_edge_cases.py` — empty collections, unicode, del, chained assignment, slices
19. `test_context_managers.py` — custom __enter__/__exit__
20. `test_comprehensions.py` — nested comprehensions, if filters
21. `test_imports.py` — import system
22. `test_re_advanced.py` — regex with groups, flags, subn
23. `test_functools_advanced.py` — lru_cache, singledispatch

### Phase 4 — Advanced / Fills (After VM Fixes)
*(Only after the corresponding VM blockers are resolved)*

24. `test_functions_advanced.py` — decorator chaining (needs CALL_FUNCTION_EX)
25. `test_generators.py` — multi-yield (needs generator fix)
26. `test_recursive_algorithms.py` — recursion, tree traversal
27. `test_oop_patterns.py` — factory, singleton, adapter
28. `test_django_import_chain.py` — simulated Django import chain
29. `test_dataclasses.py` — @dataclass (needs module complete)
30. `test_typing.py` — typing generics (needs __class_getitem__)

---

*End of Test Plan*
