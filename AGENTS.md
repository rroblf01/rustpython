# AGENTS.md

Independent Python 3.13/3.14 interpreter in Rust — own lexer, parser, AST, bytecode compiler, stack VM. Not upstream `RustPython`. See `CLAUDE.md` (architecture), `GAP_ANALYSIS.md` (measured compat vs CPython `Lib/test`), `INSTRUCTIONS.md` (cpython-suite hunt workflow + never-commit rule). Ground truth on conflicts: CPython 3.14 source at `~/Documentos/cpython/{Lib,Modules,Python,Objects,Include}`, not prose.

## Build — use Makefile, not raw cargo

```bash
make it                 # cargo check --release (~7s gate) + release-lite build; use between every edit
make dev                # release-lite (opt-level=1, no LTO, 16 codegen units) — iteration, not benchmarks
make release            # opt-level=3 thin-LTO panic=abort strip -> target/release/rustpython — sweeps/benchmarks
make build              # debug -> target/debug/rustpython
make static             # +crt-static --no-default-features — JIT disabled (dlopen)
```

`jit` is default; `sqlite3` opt-in/heavy (`cargo build --features jit,sqlite3` for feature control). `main()` runs on a 64 MB-stack thread (`src/main.rs:418`) — deep recursion is by design, not a bug to fix. No `rustfmt.toml`/`clippy.toml` — defaults.

## Test — binary, cwd and runner matter

- `make test-python` always rebuilds a **release** binary with `jit,sqlite3` as `target/debug/rustpython-test` (`build-test`, `Makefile:106-109`), runs `tests/*.py` sequentially (pass = exit 0), logs to `/tmp/rustpython-test-logs/`. A fresh `make build` does NOT satisfy it.
- `make test-one FILE=tests/test_x.py [TIMEOUT=60]` — only safe single-file runner: resolves binary/test path absolute, runs in `mktemp -d` scratch dir with `timeout --kill-after=10`, cleans up. Never run the binary on `tests/cpython/test_*.py` from repo root — tests create relative-path scratch files (`@test_*_tmp*`, `test_dbm_dir*`) and pollute `git status`.
- `make test-cpython` — vendored CPython 3.14 corpus (`tests/cpython/test_*.py`, ~398 files), 180s timeout / 12 parallel (`make test-cpython-quick` = 60s). Logs to `/tmp/rustpython-test-logs/cpython/`. Runner (`tests/cpython_runner.sh`) emits `PASS|SKIP|FAIL|TIMEOUT|PARSE_ERROR`; `SkipTest`/`ResourceDenied` in log = `SKIP`, not FAIL. Current numbers live in `GAP_ANALYSIS.md`; don't hardcode counts. A "failing" file often passes 90%+ subtests — read logs, not just file counts.
- `./test.sh -c "code"` / `./test.sh file.py` — ad-hoc runner (`BUILD_PROFILE=release|release-lite`, `FEATURES`, `TIMEOUT`, `VENV` env vars; auto-injects Django site-packages when code mentions `django`).
- Sweep safety: `make test-cpython` builds once at start. Don't edit/rebuild mid-sweep (binary gets `text file busy` contamination); don't run `make test-python` concurrently.
- Verify after every fix: `cargo check --release` clean, `cargo test`, `make test-python` — any delta is a regression. Never commit; user handles all commits (`INSTRUCTIONS.md`).

## Lint / Format

```bash
make lint       # cargo clippy -D warnings + cargo fmt --check
make lint-all   # + ruff (tests/*.py) + typos (if installed)
make lint-fix   # cargo fmt
```

## Architecture — where code lives

```
Source -> token.rs -> parser.rs -> ast.rs -> compiler.rs -> bytecode.rs -> src/vm/ -> Result
                                              ^                         |
                                              +--- src/object/ ---------+
```

- `src/` — flat top-level files, no workspace split. `src/vm/` (~34 files, re-exported via `vm.rs`): `dispatch.rs`/`execute.rs` route to `op_*.rs` by opcode family; `call.rs`/`call_func.rs`/`call_class.rs`, `import.rs`, `frame.rs`/`pool.rs`.
- `src/object/` (via `object/mod.rs`): `core/pyref.rs` (`PyObjectRef`), `core/attr_map.rs`, `attrs/` per-type (`int.rs`, `str1.rs`, `list.rs`, …), `builtins/` free functions, `ops_*.rs` dunder dispatch, `subscript/`/`pydict/` etc.
- `src/modules/` — native stdlib grouped thematically (`core/`, `crypto.rs`, `data.rs`, `dev.rs`, `files.rs`, `misc.rs` largest, `net.rs`, `text.rs`, `time.rs`, feature-gated `sqlite3.rs`). `modules/mod.rs` re-exports flat.
- `Lib/` — pure-Python stdlib vendored verbatim from CPython; takes priority over native modules via `vm/import.rs` (+ venv `site-packages` autodiscovery). Prefer vendoring pure-Python modules verbatim over Rust reimplementation — but revert if `make test-cpython` regresses; verify net PASS delta.
- New opcode: define in `bytecode.rs` → handler in matching `src/vm/op_*.rs` (find via `dispatch.rs`/`execute.rs`) → optional JIT lowering in `jit.rs` (feature `jit`).
- Keep files ~1000 lines: split `foo.rs` → `foo/bar.rs` directory module (`foo.rs` keeps glue, declares `mod bar;`). Same pattern created `src/object/` and `src/vm/`.

## Object representation — read before touching `src/object/`

`src/object/core/pyref.rs` — hand-rolled tagged enum, not uniform `Rc`:

```rust
enum PyObjectRef { SmallInt(i64), SmallBool(bool), SmallFloat(f64), SmallStr(<16 bytes), None,
                   Mut(Rc<RefCell<PyObject>>), Imm(Rc<RefCell<PyObject>>) }
```

`Mut` = List/Dict/Set/Instance; `Imm` = Int(bigint)/Str/Float/Tuple/Bytes/Code/Function. `.borrow()`/`.borrow_mut()` panic on conflict — a double-borrow is a bug. New types: pick `Mut` vs `Imm` by mutability.

## Gotchas — will break without these

- **Kwargs convention**: plain `fn(&[PyObjectRef])` builtins receive kwargs packed as a trailing `PyDict` appended to `args` (see `src/vm/call.rs`). Indexing `args[N]` positionally without a last-arg-is-dict check breaks `f(x, kw="y")`.
- **`with_vm_mut` hazard**: plain builtins reachable from live bytecode must NOT use `with_vm_mut` for `&mut VM` (aliasing UB — segfaults / empty-frame reads). Extract `fn(vm: &mut VM, …)` and intercept in `src/vm/call.rs` via `std::ptr::fn_addr_eq` (many examples there: `print`, `exec`/`eval`, `__import__`, `asyncio.run`, `globals`/`locals`, `sys.exc_info`, …).
- **`BuiltinFunction` auto-binding**: `LOAD_ATTR` auto-binds `BuiltinFunction` via class MRO to `self` — right for native type methods, wrong for free functions stored as class attrs (`class C: open = io.open`). Exclude via `fn_addr_eq` next to `is_builtin_exception_class_name` (`src/vm/op_attr.rs`, `src/object/pyobject/repr.rs`).
- **Instance-dict methods get no `self`**: `BuiltinFunction` in an instance dict is called without `self`. Native methods must live in the type dict.
- **Name mangling**: `__x` → `_ClassName__x` must apply to attribute access, `FunctionDef` storage key, and bare `Name` reads/writes in class bodies (`src/compiler/` `class_name_stack`, `mangle_name`).
- **CLI compat**: `src/main.rs:544` must accept `-X` (incl. attached `-Xfoo`), `-I`, `-E`, `-u`, `-S`, `-s`, `-W`/`-Wd`, `-i`, `-c`, `-m` — `Lib/test/support/script_helper.py` spawns subprocesses with these; rejecting them fails large parts of `test-cpython` before test code runs.
- No `serde`/`pyo3`/`tokio`; allowed deps include `num-bigint`, `regex`+`fancy-regex`, `compact_str`, `flate2`, `rustyline`, `mimalloc` (global allocator). Check `Cargo.toml` before adding a crate.
