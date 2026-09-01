# AGENTS.md

Independent Python 3.13/3.14 interpreter reimplemented in Rust — own lexer, parser, AST, bytecode compiler, stack VM. Not upstream `RustPython`. See `CLAUDE.md` (architecture), `GAP_ANALYSIS.md` (true compat signal), `ROADMAP-v2.md` (perf roadmap), `INSTRUCTIONS.md` (cpython-suite hunt workflow).

## Build — use Makefile, not raw cargo

```bash
make build              # debug -> target/debug/rustpython
make release            # release opt-level=3 thin-LTO panic=abort strip -> target/release/rustpython
make dev                # release-lite opt-level=1 no-LTO 16 codegen units — fast iteration
make it                 # cargo check --release (fast error gate) + release-lite build; use between every edit
make check              # cargo check
make static             # RUSTFLAGS=+crt-static --no-default-features — JIT disabled (dlopen)
cargo build --features jit,sqlite3  # only when you need feature control (jit is default, sqlite3 opt-in/heavy)
```

`release` profile is size-conscious (`strip`, `panic=abort`). `release-lite` exists for incremental speed.

## Test — binaries and ordering matter

- `make test` = `test-rust` + `test-python`
- `make test-rust` = `cargo test` (currently 2 Rust unit tests)
- `make test-python` builds **release** binary with `jit,sqlite3` as `target/debug/rustpython-test` (`build-test` target) then runs `tests/*.py` sequentially (pass = exit 0), logs to `/tmp/rustpython-test-logs/`. Don't assume `make build` (debug) satisfies it — `test-python`/`test-cpython` rebuild release on their own.
- `make test-one FILE=tests/test_x.py` — only safe ad-hoc single-file runner. Runs from `mktemp` scratch dir and cleans up. Never run `./target/debug/rustpython tests/cpython/test_foo.py` directly from repo root — those tests create files via relative paths (`test_dbm_dir*`, `@test_*_tmp*`, `6`..`25`) and will pollute `git status`.
- `make test-cpython` — vendored CPython 3.14 corpus `tests/cpython/test_*.py` (~398 files), `120s` timeout, `12` parallel, logs to `/tmp/rustpython-test-logs/cpython/`. `make test-cpython-quick` same with `30s` timeout. Check aggregate `PASS/FAIL/TIMEOUT/PARSE_ERROR`, not just file count — single failing files often pass 90%+ subtests.
- `test.sh` — quick runner: `./test.sh -c "code"` or `./test.sh file.py`. Env: `BUILD_PROFILE=release|release-lite`, `FEATURES=jit,sqlite3`, `TIMEOUT=10`, `VENV=/tmp/rustpython-django-env`. Auto-injects Django venv `site-packages` if code mentions `django`; uv integration uses `/tmp/test-uv-rustpython/.venv` (`make test-uv`, `make run-uv SCRIPT=x`).
- Sweep safety: `make test-cpython` does a one-time `cargo build` at start. Wait for `grep -q "cpython/test_" /tmp/rustpython-test-logs/sweepN.log` before editing/rebuilding again — mid-build edits contaminate `rustpython-test` binary. Don't run `make test-python` concurrently with a sweep (`cp: text file busy`).

Verify baseline after every fix: `cargo build 2>&1 | grep "^error"` empty, `cargo test`, `make test-python` — any change from baseline is a regression to investigate.

## Lint / Format

```bash
make lint       # cargo clippy -D warnings + cargo fmt --check
make lint-all   # + ruff (tests/*.py) + typos (if installed)
make lint-fix   # cargo fmt
```

No `rustfmt.toml`/`clippy.toml` — defaults.

## Architecture — where code lives

```
Source -> token.rs -> parser.rs -> ast.rs -> compiler.rs -> bytecode.rs -> src/vm/ -> Result
                                              ^                         |
                                              +--- src/object/ ---------+
```

- `src/` — 14 top-level `.rs` files, no workspace split. `main.rs:367` spawns `real_main` on 512MB stack thread (deep recursion guard; `vm.rs` frame-len checks).
- `src/vm/` (~34 files, re-exported via `vm.rs`): `frame.rs`/`pool.rs` (frames), `dispatch.rs`/`execute.rs` (dispatch), `op_*.rs` split by opcode family (`op_attr.rs`, `op_call.rs`, `op_exc.rs`, `op_import.rs`, `op_store.rs`, `op_var.rs`, `op_with.rs`), `call.rs`/`call_func.rs`/`call_class.rs`, `except.rs`, `class.rs`/`descriptor.rs`, `import.rs`, `format.rs`, `iter.rs`.
- `bytecode.rs` — opcode defs (CPython 3.11+ style: `RESUME`, `CALL_FUNCTION_EX`, `CALL_KW`, match opcodes). Adding an opcode: define in `bytecode.rs` -> handler in matching `src/vm/op_*.rs` (find via `dispatch.rs`/`execute.rs`) -> optional JIT lowering in `jit.rs:FEATURE(jit)`.
- `src/object/` (~19+ files, `object/mod.rs` re-exports): `core/pyref.rs` (`PyObjectRef`), `core/attr_map.rs`, `attrs/` (per-type data: `int.rs`, `str1.rs`/`str2.rs`, `bytes1.rs`/`bytes2.rs`, `list.rs`, `dict.rs`, etc.), `builtins/` (free functions), `ops_*.rs` (dunder dispatch), `descriptors.rs`, `ctors/`, `pydict/`, `subscript/`, `memoryview/`, etc.
- `src/modules/` — native stdlib, grouped thematically (`core/mod.rs`, `crypto.rs`, `data.rs`, `dev.rs`, `files.rs`, `misc.rs` (largest/catch-all), `net.rs`, `text.rs`, `time.rs`, plus `binascii.rs`, `unicodedata.rs`, `sqlite3.rs` gated). Each `foo.rs` declares `mod bar;` -> `foo/bar.rs`. `modules/mod.rs` re-exports flat.
- `Lib/` — pure-Python stdlib loaded at import time (many packages vendored verbatim from CPython). `Lib/` takes priority over native modules via `vm/import.rs` path search + venv `site-packages` autodiscovery.
- `interner.rs` — thread-local `StrId(u32)` string interner for `LOAD_NAME`/`LOAD_GLOBAL`/`LOAD_ATTR`.
- `superinstr.rs` — fused superinstructions; `cycle_gc.rs`/`gc.rs` (gc feature is experimental, default is `Rc<RefCell<PyObject>>` — leaks cycles).

## Object representation — read before touching `src/object/`

`src/object/core/pyref.rs`: hand-rolled tagged enum, not uniform `Rc`:

```rust
enum PyObjectRef { SmallInt(i64), SmallBool(bool), SmallFloat(f64), SmallStr(<16 bytes), None, Mut(Rc<RefCell<PyObject>>), Imm(Rc<RefCell<PyObject>>) }
```

`Mut` = List/Dict/Set/Instance; `Imm` = Int(bigint overflow)/Str/Float/Tuple/Bytes/Code/Function. `.borrow()`/`.borrow_mut()` panic on conflict — double-borrow is a bug. Decide Mut vs Imm by mutability.

## Conventions & Gotchas

- **File splitting**: keep files ~1000 lines. When grown, split `foo.rs` -> `foo/bar.rs` directory module (`foo.rs` keeps glue, declares `mod bar;`). Same pattern created `src/object/` and `src/vm/`.
- **Cargo features**: `jit` (Cranelift, default), `gc` (tracing GC, not default), `ffi` (`libloading`, limited), `sqlite3` (`rusqlite`, heavy), `profile` (PGO). Check `Cargo.toml` before adding crates — no `serde`/`pyo3`/`tokio`; allowed deps include `num-bigint`, `regex`+`fancy-regex`, `compact_str`, `flate2`, `rustyline`, `mimalloc` (global allocator).
- **Vendoring `Lib/`**: prefer vendoring pure-Python modules verbatim from CPython over Rust reimpl — but revert if `make test-cpython` regresses (e.g. compression modules previously reverted for -11 regressions). Verify net PASS delta.
- **Calling convention**: `BuiltinFunction`/`BuiltinMethod` packs keyword args as trailing `PyDict` appended to args. Position-only reads of `args[N]` without checking last-arg-is-kwargs-dict break `f(x, kw="y")` calls.
- **`with_vm_mut` hazard**: plain `fn(&[PyObjectRef])` builtins reachable from live bytecode must not use `with_vm_mut` to get `&mut VM` (UB). Extract `fn(vm: &mut VM, ...)` impl and intercept in `vm.rs:call_function` via `std::ptr::fn_addr_eq` — grep `fn_addr_eq` for examples.
- **`BuiltinFunction` auto-binding**: `LOAD_ATTR` auto-binds any `BuiltinFunction` found via class MRO to `self`. Correct for native type methods, wrong for free functions stored as class attrs (`class C: open = io.open`) — exclude via `fn_addr_eq` next to `is_builtin_exception_class_name` in `vm.rs`.
- **Name mangling**: `__x` -> `_ClassName__x` must apply to attribute access, `FunctionDef` storage key, and bare `Name` reads/writes inside class body (`compiler.rs:mangle_name` with `class_name_stack`).
- **CLI compat**: `main.rs:real_main` must accept `-X`, `-I`, `-E`, `-u`, `-S`, `-s`, `-W`/`-Wd`, `-i`, `-c`, `-m`/`-mmod` — `Lib/test/support/script_helper.py` spawns subprocesses with these; rejecting them fails large parts of `test-cpython` before test code runs.
- **Benchmarks**: `benchmarks/` (`bench_trend.py`, `*.py` + `.sh` vs CPython) — use these for perf, not ad-hoc microbenchmarks.
