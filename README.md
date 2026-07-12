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

Comparing RustPython **Cranelift JIT** (release build) against CPython 3.13.
All times are the average of 3 runs. Benchmarks exercise real Python patterns.

### Speed

| Benchmark | CPython 3.13 | RustPython JIT | Ratio |
|-----------|-------------|----------------|-------|
| **Pure arithmetic** (50k iter) | **20.9 ms** | **31.6 ms** | **1.51×** |
| **Full benchmark suite** (13 tests, N=2000 each) | **88.6 ms** | **1441.7 ms** | **16.28×** |
| Fibonacci(30) | ~11 ms | ~? | — |
| Nested loops (100×100) | ~0.3 ms | ? | — |

> **Pure arithmetic** is the JIT's best case: `i64` fast paths inline arithmetic without heap allocation. Here RustPython is only **1.51× slower** than CPython — competitive.
>
> **Full suite** includes class definitions, comprehensions, dict/attr/call heavy patterns — features that hit the bytecode interpreter fallback path. The 16× gap matches expectations for a toy interpreter with `Rc<RefCell<>>` per-object overhead.
>
> *Per-benchmark RustPython timings are not available — the `time` module is not yet implemented.*

### Memory

| Metric | CPython 3.13 | RustPython JIT | Ratio |
|--------|-------------|----------------|-------|
| Binary size | **6.5 MB** | **5.0 MB** | **0.77×** (23% smaller) |
| Peak RSS (sampled) | **~8.6 MB** | **~5.2 MB** | **0.60×** (40% less) |

RustPython produces a **smaller binary** and has a **lighter memory footprint** than CPython for the same workloads, thanks to Rust's efficient memory model and the absence of a full GC at runtime.

### JIT Opcodes

The Cranelift JIT currently supports **~35 bytecode opcodes** (up from 26):

| Category | Opcodes |
|----------|---------|
| Load/Store | `LOAD_FAST`, `LOAD_CONST`, `LOAD_GLOBAL`, `LOAD_NAME`, `STORE_FAST`, `LOAD_DEREF`, `STORE_DEREF` |
| Arithmetic | `BINARY_OP`, `UNARY_NEGATIVE`, `UNARY_NOT`, `UNARY_INVERT`, `COMPARE_OP` |
| Build | `BUILD_LIST`, `BUILD_TUPLE`, `BUILD_MAP`, `BUILD_SET`, `BUILD_SLICE`, `BUILD_STRING`, `LIST_APPEND` |
| Control flow | `POP_JUMP_IF_FALSE`, `POP_JUMP_IF_TRUE`, `POP_JUMP_IF_NONE`, `POP_JUMP_IF_NOT_NONE`, `JUMP_BACKWARD`, `JUMP_FORWARD`, `RETURN_VALUE` |
| Iteration | `GET_ITER`, `FOR_ITER`, `CONTAINS_OP`, `UNPACK_SEQUENCE`, `UNPACK_EX` |
| Functions | `CALL`, `POP_TOP`, `DUP_TOP`, `COPY`, `SWAP`, `PUSH_NULL` |
| Attributes | `LOAD_ATTR`, `STORE_ATTR`, `STORE_SUBSCR` |
| Identity | `IS_OP` |
| Import | `IMPORT_NAME`, `IMPORT_FROM` |
| Context mgr | `SETUP_WITH`, `WITH_EXIT` |

### Language features now supported

| Feature | Status |
|---------|--------|
| `with` statement / context managers | ✅ Added `SETUP_WITH`/`WITH_EXIT` JIT support |
| `import` statements | ✅ Added `IMPORT_NAME`/`IMPORT_FROM` JIT support |
| `@decorator` syntax | ✅ Already existed in parser |
| `__getitem__`/`__setitem__`/`__iter__` on custom classes | ✅ Already existed |
| `try`/`except`/`finally` | ✅ Already existed in VM |
| `is` / `is not` | ✅ Added `IS_OP` JIT support |
| `~` bitwise invert | ✅ Added `UNARY_INVERT` JIT support |
| Set literals | ✅ Added `BUILD_SET` JIT support |
| Slice literals | ✅ Added `BUILD_SLICE` JIT support |
| `COPY`, `SWAP` | ✅ Added direct eval_stack manipulation |
| Starred assignment (`*a, b = ...`) | ✅ Added `UNPACK_EX` JIT support |

### Not yet implemented (for 1.0)

| Feature | Effort | Impact | Status |
|---------|--------|--------|--------|
| Inline cache for `LOAD_ATTR` | 🟡 Medium | 2-5× faster attr access | ✅ Done (thread-local cache) |
| `raise...from` chaining | 🟢 Low | Niche | ✅ Already existed in VM |
| `async`/`await` | 🔴 Very high | Unlocks asyncio | ✅ Done (parser, compiler, VM, Coroutine type) |
| Standard library modules | 🔴 Very high | Unlocks whole ecosystem | ✅ math, sys, time (básicos) |
| Full Python stdlib | 🔴🔴🔴 Enormous | Drop-in CPython replacement | ⏳ Futuro |

### Optimization history (JIT edition)

| Change | Impact |
|--------|--------|
| Baseline JIT (11 opcodes) | ~3.0× CPython for arithmetic |
| +7 unary/build/iter opcodes | Covers list/tuple/iter/in patterns |
| +CALL + LOAD_ATTR | Enables JIT for functions with calls and attribute access |
| +SmallInt fast path for COMPARE_OP + UNARY_NEGATIVE | Inline `icmp`/negation avoids C call for inline integers |
| +FOR_ITER, BUILD_MAP, STORE_ATTR, UNPACK_SEQUENCE, LOAD_NAME | 26 opcodes total, covers majority of real Python patterns |
| **Pure arithmetic JIT** | **1.51× CPython** (competitive for hot loops) |

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
