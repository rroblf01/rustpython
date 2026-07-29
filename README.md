# RustPython

A Python 3 interpreter reimplemented from scratch in Rust — its own lexer, parser, AST,
bytecode compiler, and stack-based VM, plus a large surface of the CPython standard library
reimplemented natively in Rust. Target: CPython 3.13/3.14 language compatibility.

(This is an independent codebase, not affiliated with the upstream `RustPython` GitHub project.)

## What is this?

- **Lexer** (`src/token.rs`) — tokenizes Python source
- **Parser** (`src/parser.rs`) — hand-written recursive-descent parser, AST output
- **Compiler** (`src/compiler.rs`) — AST → bytecode
- **VM** (`src/vm.rs`) — stack-based bytecode interpreter
- **Object system** (`src/object/`, a ~19-file directory module) — the `PyObjectRef`/`PyObject`
  type hierarchy, attribute dispatch, and every builtin type's behavior
- **`src/modules/`** — native (Rust) standard library modules, split thematically
- **`Lib/`** — a smaller set of stdlib modules implemented in pure Python where that's easier
  than a native reimplementation (`asyncio/`, `http/`, `importlib/`, `tarfile.py`, etc.)

## Current status

There's no honest single "% compatible" number for this project — see `GAP_ANALYSIS.md` for
the full, current breakdown (and why the old headline "~98%" claim didn't hold up once measured
against CPython's own test suite). The short version:

**Solid**: core language semantics (classes, closures, generators, exceptions, most operators,
`match`/`case`, f-strings), all 12 practically-migratable builtin types (`int`/`str`/`list`/
`float`/`dict`/`tuple`/`bytes`/`set`/`complex`/`bytearray`/`frozenset`/`bool`) are real,
subclassable `Type` objects (`type(5) is int` holds), a correct rich-comparison dispatch
protocol, and broad native stdlib module coverage (`math`, `json`, `re`, `os`, `re`, `hashlib`,
`socket`, `itertools`, `functools`, `enum`, and more).

**Known, real, architectural gaps** (not "a few more bugs" — see `GAP_ANALYSIS.md` for detail):
no real multi-threading safety, no cycle-collecting GC (`Rc<RefCell<>>` leaks reference cycles),
C extension loading is effectively non-functional, `asyncio` has no real event loop, and several
stdlib modules (`ctypes`, `multiprocessing`, `unittest.mock`, a real `tokenize`/`ast`, ...) are
stubs or missing.

**How this is measured**: `make test-cpython` runs all 398 real CPython 3.14 `Lib/test/` files
(vendored in `tests/cpython/`) against the interpreter — this is the primary compatibility
signal, tracked continuously; `make test-python` is a small hand-written smoke-test suite kept
at a stable baseline every change must not regress.

## Building

```bash
make build              # debug build -> target/debug/rustpython
make release            # release build (opt-level=z, thin LTO) -> target/release/rustpython
make static             # fully static binary (JIT disabled)
```

Direct `cargo` also works: `cargo build [--release] [--features jit,sqlite3]`.

## Running

```bash
./target/debug/rustpython script.py     # run a file
./target/debug/rustpython               # REPL
./target/debug/rustpython -c "..."      # -c
```

### uv / venv integration

Run from inside a directory with `.venv/` (e.g. created by `uv sync`) and the interpreter
auto-detects `VIRTUAL_ENV`/`.venv/` and adds the venv's `site-packages` to `sys.path`:

```bash
cd my-project
uv init && uv add requests
/path/to/rustpython -c "import requests; print(requests.__version__)"
```

## Testing

```bash
make test               # test-rust + test-python
make test-rust          # cargo test
make test-python        # runs tests/*.py against the debug binary
make test-cpython       # runs the full vendored CPython 3.14 test corpus (~398 files, slower)
make test-cpython-quick # same, with a shorter per-file timeout — for quick iteration
make test-one FILE=tests/test_x.py   # a single hand-written test file
```

`test.sh` is a quick ad-hoc runner: `./test.sh -c "code"` or `./test.sh file.py`.

## Cargo feature flags

- `jit` — Cranelift-based JIT for hot bytecode (~35 opcodes covered; falls back to the bytecode
  interpreter otherwise)
- `gc` — experimental generational tracing GC (`src/gc.rs`); **not the default**, `Rc<RefCell<>>`
  is what actually runs today
- `ffi` — loads real CPython C-extension `.so` files (limited; see `GAP_ANALYSIS.md`)
- `sqlite3` — native `sqlite3` module backed by `rusqlite`
- `profile` — execution profiling for PGO-guided JIT decisions

## Dependencies

Zero-dependency was an original design goal; real dependencies have been added where
reimplementing was impractical: `num-bigint`/`num-traits` (arbitrary-precision integers),
`regex`, `smallvec`, `once_cell`, `compact_str`, `md-5`/`sha-1`/`sha2`, `flate2`, `rustyline`
(REPL), `mimalloc`. No `serde`, no `pyo3`, no `tokio`.

## Architecture

```
Source → Lexer → Tokens → Parser → AST → Compiler → Bytecode → VM → Result
                              ↑                                        |
                              └──────────── Object System ─────────────┘
```

See `CLAUDE.md` for the full architectural writeup (module-by-module breakdown, object
representation details, working conventions).
