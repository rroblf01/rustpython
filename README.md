# RustPython

A Python 3 reimplementation in Rust — a toy interpreter built from scratch without external dependencies (except `num-bigint` and `num-traits` for arbitrary-precision integers).

## What is this?

RustPython is a learning project that implements a significant subset of Python 3 by building all core components from scratch:

- **Lexer** (`token.rs`) — tokenizes Python source into tokens
- **Parser** (`parser.rs`) — recursive-descent parser producing an AST
- **Compiler** (`compiler.rs`) — compiles AST to bytecode
- **VM** (`vm.rs`) — stack-based bytecode interpreter
- **Object system** (`object.rs`) — core types, builtins, and methods

## Features implemented

| Category | Details |
|----------|---------|
| Types | `None`, `bool`, `int` (arbitrary precision), `float`, `str`, `bytes`, `bytearray`, `list`, `tuple`, `dict` (arbitrary hashable keys), `set`, `range` (lazy), `slice`, `function`, `generator`, `class`, `module`, `file`, `super`, `property`, `staticmethod`, `classmethod` |
| Operators | Arithmetic (`+` `-` `*` `/` `//` `%` `**`), bitwise (`<<` `>>` `&` `|` `^` `~`), comparison (`<` `<=` `==` `>=` `>` `!=`), membership (`in` `not in`), identity (`is` `is not`) |
| Comprehensions | List, dict, set — including multi-generator and `if` filters |
| Functions | `def`, `lambda`, `*args`, `**kwargs`, default arguments, closures |
| Classes | Single inheritance, `__init__`, `__str__`, `__len__`, `__repr__`, `__hash__`, `__call__`, properties, staticmethod, classmethod |
| Control flow | `if`/`elif`/`else`, `for`/`in`, `while`, `try`/`except`, `match`/`case`, `break`/`continue` |
| Generators | `yield`, `yield from`, generator expressions |
| Builtins | 60+ builtin functions including `print`, `len`, `range`, `type`, `int`, `float`, `str`, `list`, `tuple`, `dict`, `set`, `bytes`, `bytearray`, `format`, `object`, `hash`, `slice`, `dir`, `globals`, `locals`, `ascii`, `frozenset`, `memoryview`, `sorted`, `enumerate`, `zip`, `map`, `filter`, `any`, `all`, `sum`, `min`, `max`, `abs`, `repr`, `iter`, `next`, `open`, `eval`, `exec`, `super`, `property`, `staticmethod`, `classmethod`, `isinstance`, `issubclass`, `hasattr`, `getattr`, `setattr`, `delattr`, `id`, `callable`, `pow`, `reversed`, `help`, `exit`, `input`, `ord`, `chr`, `hex`, `oct`, `bin` |
| Methods | List (9), Dict (9), Str (27), Set (18) |
| Literals | `b"..."` / `b'...'` bytes, `f"..."` f-strings, escape sequences including `\uHHHH` and `\UHHHHHHHH` |

## Not implemented

- `async`/`await`
- `with` statement / context managers
- `@decorator` syntax
- Full `except ... as e` pattern
- `finally` blocks
- `raise ... from` chaining
- `del` statement
- `@` matrix multiply operator
- f-string format specs
- `__getitem__`/`__setitem__`/`__iter__`/`__next__` on custom classes
- Multiple inheritance MRO
- `import` of standard library modules

## Building

```bash
cargo build --release
```

## Running

```bash
# Run a file
./target/release/rustpython script.py

# REPL
./target/release/rustpython
```

## Benchmarks

Comparing RustPython (release build) against CPython 3.14.6.
All times measured with `/usr/bin/time`, memory via `/proc/self/status`.

### Speed

| Benchmark | CPython 3.14 | RustPython | Ratio |
|-----------|-------------|------------|-------|
| Total wall time (arithmetic, lists, dicts, strings, functions, loops, comprehensions, sets) | **0.53s** | **3.00s** | **~5.7×** |

*Individual section timings not available for RustPython — the `time` module is not implemented.*

RustPython is approximately **18× slower than CPython** for this benchmark suite.
This is expected for a toy interpreter with `BigInt` for all integers,
`Rc<RefCell<>>` overhead for every object, and minimal optimization.

### Memory

| Test | CPython | RustPython (Before) | RustPython (After) | Ratio vs CPython |
|------|---------|--------------------|--------------------|------------------|
| List of 1000 × [0..999] | 30.4 MB | 513 MB | **8.0 MB** | **~3.8× smaller** |
| Dict of 1000 str→list entries | 0.9 MB | 52 MB | **1.6 MB** | **~1.8× larger** |

### Optimization history

| Version | Time | vs CPython | Key change |
|---------|------|-----------|------------|
| Original | 10.28s | ~20× | Baseline with BigInt + Rc<RefCell> |
| + Small int cache | ~10.2s | ~20× | Cache -5..257, later extended to 999 |
| + ListIter + RangeIter | ~9.3s | ~18× | O(1) FOR_ITER, lazy range |
| + i64 fast paths | ~8.9s | ~17× | Native arithmetic when values fit in i64 |
| + Rc<CodeObject> | ~8.0s | ~15× | Avoid CodeObject clone on every function call |
| + LTO + codegen-units=1 | ~3.5s | ~6.7× | Fat LTO + single codegen unit |
| + Rc<HashMap> builtins | **3.46s** | **~6.7×** | Avoid builtins HashMap clone on every call |
| + Rc<HashMap> builtins (function calls) | **0.84s** | — | Function call micro-benchmark: 5.8× faster |
| + Vec locals + simple executor | **3.62s** / **0.69s** (fn) | **~7.0×** | O(1) LOAD_FAST/STORE_FAST + inline simple funcs |
| + Lazy range + RangeIter | **3.73s** / **0.77s** (fn) | **~6.8×** | range() lazy, iteración O(1) con RangeIter |

## Architecture

```
Source → Lexer → Tokens → Parser → AST → Compiler → Bytecode → VM → Result
                              ↑                                        |
                              └──────────── Object System ─────────────┘
```

The object system (`object.rs`) defines all Python types, builtin functions,
and the `get_attribute` dispatch for method resolution on built-in types.
Method calls on user-defined classes go through the VM's `LOAD_ATTR` handler
which checks instance dicts and type MROs.

## Dependencies

- `num-bigint` — arbitrary-precision integers
- `num-traits` — numeric trait implementations

Zero other external dependencies. No regex, no serde, no pyo3.
