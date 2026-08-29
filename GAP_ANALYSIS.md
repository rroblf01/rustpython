# RustPython — CPython 3.14 Compatibility Gap Analysis

Last updated: August 29, 2026 (previous version dated July 29 — badly stale: ~260 commits landed
between the two, moving PASS from 22 to 93; the module-file split into `src/vm/`/`src/object/`/
`src/modules/*/` subdirectories, the Lib/ pure-Python vendoring push, and most of the "known deep
gaps" list below happened in that window too — see `CLAUDE.md` for the current architecture and
the `cpython_test_suite_compat` memory topic for the full batch-by-batch history).

## How "compatibility" is actually measured here

There is no single trustworthy percentage for "how done is this interpreter." What exists is:

- **`make test-cpython`**: runs all 398 real CPython 3.14 stdlib test files (vendored verbatim
  in `tests/cpython/`, see `tests/cpython/README.md`) against a release build. This is the
  primary, most objective signal.
- **`make test-python`**: a small hand-written regression suite (`tests/*.py`), currently
  **18 passed / 16 failed** — kept stable as a smoke-test baseline every change must not regress.

Latest full `make test-cpython` sweep (2026-08-29): **94 / 398 files pass with zero failures**
(299 FAIL, 5 TIMEOUT), up from 22 on July 29. File-level pass/fail counts are a *harsh*
metric — CPython's own test files are exhaustive edge-case suites, so a "failing" file is very
often passing 90%+ of its individual subtests and failing on a handful of specific edge cases,
not fundamentally broken. Treat the aggregate failures+errors count as the more meaningful trend
line, and expect it to rise temporarily whenever a fix unblocks a file that was previously
crashing/hanging early — more of the file's real, previously-unreached assertions get a chance to
run and fail on their own separate (usually narrower) gaps. That is forward progress, not a
regression, and has been the dominant pattern change-over-change this project.

At least 2 files (`test_set`, `test_listcomps`) have a genuinely **non-deterministic** failure
mode — confirmed by running the exact same unmodified binary twice and getting a fast FAIL once,
a 120s TIMEOUT another time, and (for `test_set`) a `RefCell already borrowed on instance` panic
a third time. Likely tied to `PYTHONHASHSEED`-randomized hashing affecting set/dict iteration
order (see the SipHash work in git history) hitting different code paths run to run — not
(re)triggered by any specific recent change, confirmed by reproducing all three outcomes against
a build from before today's session too. Treat a TIMEOUT or panic on either of these two files as
inconclusive on its own; rerun before attributing it to a change.

Separately, `test_pprint`/`test_difflib` pass reliably (3/3) when run standalone but sometimes
FAIL under `make test-cpython`'s 12-way parallel sweep — a second, apparently distinct flakiness
class tied to CPU-contention/timing under parallel load rather than hash randomization. When
verifying a fix for a timing- or docstring-heavy test, rerun it standalone a few times rather than
trusting a single parallel-sweep result.

## What's solid

- Core language semantics: control flow, functions/closures, classes (single + multiple
  inheritance, C3 MRO), descriptors, `@property`/`@classmethod`/`@staticmethod`, `super()`,
  generators (`send`/`throw`/`close`), most operators, exception handling
  (`try`/`except`/`else`/`finally`, `raise ... from`, `ExceptionGroup`/`except*`), `with`
  statement, `match`/`case` (common patterns), f-strings, decorators.
- **All 12 practically-migratable builtin types are real `PyObject::Type` objects**: `int`,
  `str`, `list`, `float`, `dict`, `tuple`, `bytes`, `set`, `complex`, `bytearray`, `frozenset`,
  `bool`. `type(x) is X` holds correctly for every one of them, and transparent native-base
  subclassing (`class Foo(dict): ...`) works for all except `bool` (deliberately blocked with a
  real `TypeError`, matching CPython exactly — `bool` cannot be subclassed there either).
- Rich comparison protocol (`==`/`!=`/`<`/`<=`/`>`/`>=`) implements CPython's actual dispatch
  algorithm: subclass-reflected-method priority, each dunder called at most once,
  `NotImplemented` propagation, identity fallback for eq/ne.
- `object_system` (now `src/object/`, a ~19-file directory module, ~14,600 lines total — see
  below) has been hardened against several classes of reentrancy panic: a hostile
  `__eq__`/`__hash__` that mutates the dict/set/heap it's being compared within no longer
  crashes the process for the operations this was tested against (`d[k]=v`, `setdefault`,
  `set.add`/`update`, the `in` operator, `&`/`|`/`^`/`-`, comparisons, `heapq`).
- Broad native (Rust-implemented) stdlib module coverage: `math`, `json`, `re`, `os`, `sys`,
  `hashlib`, `struct`, `array`, `socket`, `subprocess`, `threading` (object model only — see
  below for real concurrency), `itertools`, `functools`, `collections`, `enum`, and many more —
  see `src/modules/` for the current file-by-file split.

## Known deep gaps (architectural, not "a few more bugs")

These are not quick fixes — each needs its own dedicated investigation/design, not an
incremental patch:

- **No real multi-threading safety.** The object model is `Rc<RefCell<PyObject>>`-based with no
  thread-safe interior mutability. `threading.Thread` can spawn real OS threads, but concurrent
  access to a shared object from two threads is unsound (confirmed panic:
  `test_itertools.py::test_count_threading`). Fixing this needs either a GIL-equivalent
  serialization point or a genuine `Arc<Mutex<>>`-based redesign of at least the objects reachable
  from thread targets. (`test_set` also panics intermittently with a `RefCell already borrowed`
  message, but reproduces even single-threaded — see the non-determinism note above; not
  confirmed to be the same root cause as this one.)
- **No cycle-collecting GC.** `Rc<RefCell<>>` reference counting never frees reference cycles
  (very common in real Python — e.g. any doubly-linked structure, many ORM-style object graphs).
  `src/gc.rs` has an experimental generational/tracing GC design, but it is **not wired in** as
  the default allocation strategy.
- **C extension loading is effectively non-functional.** `src/ffi_bridge.rs` (feature `ffi`)
  attempts `.so` loading via `libloading`, but has no `PyArg_ParseTuple`/`Py_BuildValue`
  equivalent, only understands one specific `.so` naming convention, and will very likely crash
  on any real, non-trivial C extension.
- **`asyncio` has no real event loop** — `async def`/`await`/`async for`/`async with` all parse
  and the `coroutine` type exists, but there's nothing actually driving concurrent tasks.
- **Several stdlib modules are stubs or entirely missing**: `multiprocessing`, `ctypes`,
  `unittest.mock`, `venv`, `ensurepip`, `zipimport`, `pkgutil`, most of `pdb`/`profile`/
  `cProfile`, a real `tokenize` (the current one is a hand-rolled simplified reimplementation
  with known bugs — see `cpython_test_suite_compat` memory notes on why vendoring real
  CPython's `tokenize.py` doesn't work directly for 3.12+), `ast` (only `literal_eval`),
  `compileall`/`py_compile`, `contextvars.Context` (isolated per-context storage; the current
  `ContextVar` uses one global thread-local stack), a real event-loop-backed `asyncio`,
  **`selectors`** (`Lib/selectors.py` is explicitly a no-op stub — `register`/`select`/etc. do
  nothing; needs a real `select()`/`poll()`-backed implementation), and **`io.BufferedReader`/
  `BufferedWriter`/`BufferedRandom`** (empty classes inheriting from `BufferedIOBase` with no
  actual buffering/read/write logic at all). Found 2026-07-29 chasing `test_selectors.py`.
- **`range()`'s internal representation (`PyObject::Range { start, stop, step }`) uses plain
  `i64`, not arbitrary precision.** Real CPython's `range` supports bignum-scale bounds (even
  though iterating that many times is impractical); `test_range.py` deliberately exercises
  `range(10 * sys.maxsize)`-scale values and fails regardless of any indexing/construction fix
  since they overflow `i64` outright. Fixing this needs `Range` redefined over `BigInt`
  throughout (construction, `len()`, indexing, slicing, iteration) — a large, cross-cutting
  change for a narrow real-world benefit (this is almost entirely a CPython-test-suite-only
  concern, not something real code relies on). Found/assessed 2026-07-29, deliberately deferred.

## Opcodes

All CPython 3.11+ opcodes needed for normal program execution have VM handlers. A handful of
niche/optimization-only opcodes remain unhandled (`CALL_INTRINSIC_1`/`_2`, `GET_LEN`,
`MATCH_MAPPING`/`MATCH_SEQUENCE`/`MATCH_KEYS`, `UNPACK_SEQUENCE_TWO_TUPLE`) — these are dedicated
fast-path/optimization forms of functionality that already works via a slower, already-handled
opcode sequence, not missing capability.

## Where to look for more detail

- `cpython_test_suite_compat` and `native_types_as_real_types` (Claude's persistent memory,
  outside this repo) have the blow-by-blow history of every bug found/fixed chasing the CPython
  test corpus, including exact root causes and fixes — far more detail than belongs in this file.
- `tests/cpython/README.md` for how to run a single CPython test file directly.
- `ROADMAP-v2.md` for the performance/architecture roadmap (JIT, GC, memory layout) — a
  different axis from language/stdlib *correctness* covered here.
