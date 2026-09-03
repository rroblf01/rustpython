# AGENTS.md

Independent Python 3.13/3.14 interpreter in Rust — own lexer, parser, AST, bytecode compiler, stack VM. Not upstream `RustPython`. See `CLAUDE.md` (full architecture), `GAP_ANALYSIS.md` (measured compat vs CPython `Lib/test`), `ROADMAP-v2.md` (perf roadmap), `INSTRUCTIONS.md` (cpython-suite hunt workflow).

## Goal — 100% CPython 3.14 replacement

Explicit user goal: **pass 100% of the CPython 3.14 test corpus** (`tests/cpython/test_*.py`, 398 files vendored from `~/Documentos/cpython` branch 3.14 — CPython 3.14.6 source at `~/Documentos/cpython/{Lib,Modules,Python,Objects,Include}` is the executable ground truth; trust it over prose when docs conflict). Current honest signal (full sweep 2026-09-03, quick 60s, no `exit(0)` hack): **108 PASS / 65 SKIP / 218 FAIL / 7 TIMEOUT** — commit 6dec504's `exit(0)` hacks were only PARTIALLY removed; 24 more files (argparse, math, str, datetime, subprocess, ...) still exited silently (fake PASS) plus 6 doctest-neuters, all removed in 2d29b16 (prior 131 PASS was inflated by 23). 7 TIMEOUT are newly-honest big suites exceeding 60s under 12x load (argparse, mailbox, set, signal, subprocess, userstring + pickletools). Real fixes this session: disposable-VM pooling (test_codecs >100s→5s), deferred codec registration (fixed test_dis segfault from clone approach), sys.monitoring, opcode/dis maps, version_info structseq, sys.intern, decimal Subnormal/flags, pickle opcodes, sys.modules cleanup, cmath sqrt sign, logging stubs, _collections alias. Perf baseline: uniform ~15x VM overhead (no 1000x pathology left); tail is gzip/io/pickletools ~42s. Real fixes this session: runner `SkipTest`→`SKIP` (Makefile:205), `test_fstring` nested f-string+format-spec, `test_scope` `nonlocal`/`global` in class, `platform.win32_is_iot`, `os.terminal_size`, `sys.audit`/`addaudithook`, `os.DirEntry`/`_walk_symlinks_as_files`, `zlib.DEFLATED`/`adler32`/`crc32`/`error`+`Z_SYNC_FLUSH`, `shutil` vendored `Lib/shutil.py`+`ExecError`, `struct.calcsize` bytes+`Struct`, `zipfile` vendored, `HTTPStatus.__members__`, `asyncio.events._set_event_loop_policy`, `threading._CRLock`/`_local`, `uuid._GETTERS`, `sysconfig.get_paths`, `traceback.extract_stack`, `importlib.util.spec_from_file_location`, `smtplib.LMTP`, plus FAIL→PASS/SKIP wave: `-X utf8` combined-arg CLI, 121 `Lib/encodings/*.py` vendored, codec re-entrant borrow fix, `sys.monitoring` PEP 669 stub (test_monitoring PASS), `opcode.opmap` real values, `sys.version_info` structseq+comparisons, `sys.intern`, `sys.monitoring`, `decimal Subnormal`+`Context.flags`, `test_tools` vendor (test_clinic SKIP), `spec_from_file_location` FileNotFoundError, pickle opcodes+`_dump` (test_copyreg PASS), failed-import sys.modules cleanup (test_bz2/lzma SKIP), cmath sqrt signed zero, `dis.opmap`, logging stubs, `_collections` alias, ResourceDenied→SKIP runner. Remaining 200 are triaged below — fix order: `Lib/` vendoring → parser/compiler (scope) → native `src/modules/`/C-ext stubs → threading/GC architecture. `make test-python`: 18 pass / 16 fail (all 16 pre-existing missing-Django-venv env failures).

## Build — use Makefile, not raw cargo

```bash
make build              # debug -> target/debug/rustpython
make release            # release opt-level=3 thin-LTO panic=abort strip -> target/release/rustpython (Cargo.toml:6-11)
make dev                # release-lite opt-level=1 no-LTO 16 codegen units -> target/release-lite/rustpython
make it                 # cargo check --release (fast error gate ~7s) + release-lite build; use between every edit
make check              # cargo check
make static             # RUSTFLAGS=+crt-static --no-default-features — JIT disabled (dlopen)
cargo build --features jit,sqlite3  # only when you need feature control (jit default, sqlite3 opt-in/heavy)
```

`release-lite` is for iteration speed; `release` for sweep/benchmark.

## Test — binary, cwd and runner matter

- `make test` = `test-rust` (`cargo test`, 2 Rust unit tests) + `test-python`.
- `make test-python` rebuilds **release** binary with `jit,sqlite3` as `target/debug/rustpython-test` (`build-test` target, Makefile:106-109) then runs `tests/*.py` sequentially (pass = exit 0). Logs to `/tmp/rustpython-test-logs/`. `make build` (debug) does NOT satisfy it.
- `make test-one FILE=tests/test_x.py` — only safe single-file runner. Resolves paths to absolute, runs from `mktemp -d /tmp/rustpython-test-one-scratch.*` and cleans up (Makefile:143-157). Never run `./target/debug/rustpython tests/cpython/test_foo.py` from repo root — CPython tests create files via relative paths (`@test_*_tmp*`, `6`..`25`, `test_dbm_dir*`) and pollute `git status` (see `tests/cpython_runner.sh:24-38` and `.gitignore`).
- `make test-cpython` — vendored CPython 3.14 corpus `tests/cpython/test_*.py` (~398 files), `120s` timeout, `12` parallel (`CPYTHON_TIMEOUT`/`CPYTHON_PARALLEL`, Makefile:171-183), logs to `/tmp/rustpython-test-logs/cpython/`. `make test-cpython-quick` same with `60s` (was 30s — too short for `test_heapq`/`test_gzip`/`test_io` which need 25-28s alone, 30s+parallel load caused `TIMEOUT`). Check aggregate `PASS/FAIL/TIMEOUT/PARSE_ERROR` — single failing files often pass 90%+ subtests.
- `test.sh` — ad-hoc: `./test.sh -c "code"` or `./test.sh file.py`. Env: `BUILD_PROFILE=release|release-lite`, `FEATURES=jit,sqlite3`, `TIMEOUT=10`, `VENV=/tmp/rustpython-django-env`. Auto-injects Django `site-packages` if code mentions `django`; uv integration via `/tmp/test-uv-rustpython/.venv` (`make test-uv`, `make run-uv SCRIPT=x`).
- **Runner now emits `SKIP`**: `tests/cpython_runner.sh:38-55` detects `SkipTest` in log and emits `SKIP|` + exits 0; `Makefile:205-213` counts `SKIP` separately (30 SKIP in latest sweep). Previously counted as FAIL — don't re-triage `test_win*`/`test_zstd` etc. as real failures.
- Sweep safety: `make test-cpython` does one `cargo build` at start. Don't edit/rebuild mid-sweep — binary gets contaminated (`cp: text file busy`). Don't run `make test-python` concurrently with a sweep.

Verify after every fix: `cargo build 2>&1 | grep "^error"` empty, `cargo test`, `make test-python` — any delta is a regression.

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

- `src/` — 14 top-level `.rs` files, no workspace split. `src/main.rs:368` spawns `real_main` on 64MB stack thread (deep recursion guard).
- `src/vm/` (~30 files, re-exported via `vm.rs`): `frame.rs`/`pool.rs`, `dispatch.rs`/`execute.rs`, `op_*.rs` by opcode family (`op_attr.rs`, `op_call.rs`, `op_exc.rs`, etc.), `call.rs`/`call_func.rs`/`call_class.rs`, `import.rs`, `format.rs`, `iter.rs`.
- `bytecode.rs` — opcode defs (CPython 3.11+ style: `RESUME`, `CALL_KW`, match opcodes). New opcode: define in `bytecode.rs` -> handler in matching `src/vm/op_*.rs` (find via `dispatch.rs`/`execute.rs`) -> optional JIT lowering in `jit.rs:FEATURE(jit)`.
- `src/object/` (re-exported via `object/mod.rs`): `core/pyref.rs` (`PyObjectRef`), `core/attr_map.rs`, `attrs/` per-type (`int.rs`, `str1.rs`, `list.rs`, etc.), `builtins/` free functions, `ops_*.rs` dunder dispatch, `subscript/`/`pydict/`/`memoryview/` etc.
- `src/modules/` — native stdlib, grouped thematically (`core/mod.rs`, `crypto.rs`, `data.rs`, `dev.rs`, `files.rs`, `misc.rs` largest, `net.rs`, `text.rs`, `time.rs`, plus `binascii.rs`/`sqlite3.rs` gated). Each `foo.rs` declares `mod bar;` -> `foo/bar.rs`. `modules/mod.rs` re-exports flat.
- `Lib/` — pure-Python stdlib vendored verbatim from `~/Documentos/cpython/Lib/` (`135` vs `153` `.py` files plus ~15 dir-packages), loaded at import time. `Lib/` takes priority over native modules via `vm/import.rs` + venv `site-packages` autodiscovery. For 100% goal, vendor from `~/Documentos/cpython` directly, not rewrite — see delta list in investigation (`diff <(ls ~/Documentos/cpython/Lib/*.py) <(ls Lib/*.py)` shows `base64`, `bisect`, `colorsys`, `contextvars`, `datetime`, `decimal`, `fnmatch`, `fractions`, `hashlib`, `operator`, `queue`, `shelve`, `signal`, `socket`, `ssl`, `struct`, `threading`, `warnings` etc. still missing/diverged).
- **Remaining 215 FAIL — honest triage for 100%** (`/tmp/fail_curr.txt` at 130 PASS):
  - Pure-Python `Lib/` gaps (~60): `test_shutil`, `test_tarfile`, `test_site`, `test_pdb`, `test_unicodedata` (`ucd_3_2_0`), `test_typing`, `test_array` (now `count`/`clear`/`index` added, still missing `__setitem__`/`__delitem__` subscript dispatch) — vendor/fix from `~/Documentos/cpython/Lib/` + ensure `Lib/` priority over native or extend native.
  - C-extension stubs (~80): `test_ssl` (`_ssl`), `test_symtable` (`_symtable`), `test_zlib`, `test_socket` (only `AF_INET`), `test_posix`/`test_pwd`, `test_tcl`/`test_ttk*`/`test_turtle`, `test_bz2`/`test_lzma` — need `src/modules/` impl or `ffi`/`libloading` shim matching `~/Documentos/cpython/Modules/*.c` + `Include/*.h`.
  - Parser/compiler (scope) (~20): `test_scope` (`nonlocal x` in class now parses but `testNonLocalClass` still `0 !=1`), `test_patma`, `test_peepholer` — need `compiler/closure.rs` cell/freevar handling for class scopes + `compiler.rs`. JIT: `test_dis` SIGSEGVs in `jit_make_function` cloning a corrupt PyObjectRef from JIT-compiled code (eval-stack desync; debug+gdb confirmed) — needs JIT audit, was FAIL before too so no PASS lost.
  - Architectural (~50): `test_threading`, `test_threading_local`, `test_types` (`RefCell already borrowed on Mapping` — `src/object/core/pyref.rs:137`), `test_traceback`/`test_sys_settrace`, `test_array` subscript, `test_re` — require `Arc<RwLock>`/GIL or `gc` (`ROADMAP-v2.md:9`) not quick fixes.
- `interner.rs` — thread-local `StrId(u32)` interner for `LOAD_NAME`/`LOAD_GLOBAL`/`LOAD_ATTR`.
- `superinstr.rs` — fused superinstructions; `cycle_gc.rs`/`gc.rs` (gc feature experimental, default is `Rc<RefCell<PyObject>>` — leaks cycles).

## Object representation — read before touching `src/object/`

`src/object/core/pyref.rs:31-38` — hand-rolled tagged enum, not uniform `Rc`:

```rust
enum PyObjectRef { SmallInt(i64), SmallBool(bool), SmallFloat(f64), SmallStr(<16 bytes), None, Mut(Rc<RefCell<PyObject>>), Imm(Rc<RefCell<PyObject>>) }
```

`Mut` = List/Dict/Set/Instance; `Imm` = Int(bigint overflow)/Str/Float/Tuple/Bytes/Code/Function. `.borrow()`/`.borrow_mut()` panic on conflict — double-borrow is a bug. New types: pick `Mut` vs `Imm` by mutability.

## Gotchas — will break without these

- **File splitting**: keep files ~1000 lines. When grown, split `foo.rs` -> `foo/bar.rs` directory module (`foo.rs` keeps glue, declares `mod bar;`). Same pattern created `src/object/` and `src/vm/`.
- **Cargo features** (`Cargo.toml:20-26`): `jit` default (Cranelift, `jit.rs`), `gc` experimental, `ffi` (`libloading`, limited), `sqlite3` (`rusqlite`, heavy), `profile` (PGO). No `serde`/`pyo3`/`tokio`; allowed deps include `num-bigint`, `regex`+`fancy-regex`, `compact_str`, `flate2`, `rustyline`, `mimalloc` (global allocator).
- **Vendoring `Lib/`**: prefer vendoring pure-Python modules verbatim from CPython over Rust reimpl — but revert if `make test-cpython` regresses (compression modules previously reverted for -11 regressions). Verify net PASS delta.
- **Calling convention**: `BuiltinFunction` packs kwargs as trailing `PyDict` appended to `args` (`src/vm/call.rs:401`, `src/object/builtins.rs:629`). Position-only `args[N]` without last-arg-is-dict check breaks `f(x, kw="y")`.
- **`with_vm_mut` hazard**: plain `fn(&[PyObjectRef])` builtins reachable from live bytecode must not use `with_vm_mut` for `&mut VM` (UB). Extract `fn(vm: &mut VM, ...)` and intercept in `vm.rs:call_function` via `std::ptr::fn_addr_eq` — grep `fn_addr_eq` in `src/vm/call.rs` for examples.
- **`BuiltinFunction` auto-binding**: `LOAD_ATTR` auto-binds `BuiltinFunction` via class MRO to `self`. Correct for native type methods, wrong for free functions stored as class attrs (`class C: open = io.open`) — exclude via `fn_addr_eq` next to `is_builtin_exception_class_name` in `vm.rs`.
- **Name mangling**: `__x` -> `_ClassName__x` must apply to attribute access, `FunctionDef` storage key, and bare `Name` reads/writes inside class body (`src/compiler.rs:49` `class_name_stack`, `mangle_name`).
- **CLI compat**: `src/main.rs:494-604` must accept `-X`, `-I`, `-E`, `-u`, `-S`, `-s`, `-W`/`-Wd`, `-i`, `-c`, `-m` — `Lib/test/support/script_helper.py` spawns subprocesses with these; rejecting them fails large parts of `test-cpython` before test code runs.
- **BuiltinFunction in instance dicts**: `BuiltinFunction` objects stored in instance dicts do NOT receive `self` when called. Methods must be in the type dict, not instance dict. This broke zlib `compress`/`copy` until methods were moved to type dict.
- **struct.calcsize bytes support**: `struct.calcsize()` must accept both `str` and `bytes` as format strings. CPython's `zipfile` module passes bytes format strings like `b"<4s4H2LH"`. Fixed in `src/modules/misc/struct_heapq/py_struct.rs:518`.
- **struct.Struct class**: CPython's `struct.Struct` class was missing. Added as a wrapper that stores the format string and delegates to module functions. Used by `zipfile` module.
- **zipfile module**: Native `zipfile` was limited to read-only. Vendored `Lib/zipfile/__init__.py` from CPython 3.14 for full read/write support. Native registration commented out in `src/vm/init.rs:583`.
