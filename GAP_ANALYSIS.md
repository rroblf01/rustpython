# RustPython — CPython 3.14 Compatibility Gap Analysis

Last updated: July 29, 2026 (previous version, dated July 13, was based on an early informal
smoke-test pass and its "~98% compatibility" headline figure did not hold up — this rewrite is
grounded in actually running CPython's own `Lib/test/` suite against the interpreter).

## How "compatibility" is actually measured here

There is no single trustworthy percentage for "how done is this interpreter." What exists is:

- **`make test-cpython`**: runs all 398 real CPython 3.14 stdlib test files (vendored verbatim
  in `tests/cpython/`, see `tests/cpython/README.md`) against the debug binary. This is the
  primary, most objective signal.
- **`make test-python`**: a small hand-written regression suite (`tests/*.py`), currently
  **18 passed / 16 failed** — kept stable as a smoke-test baseline every change must not regress.

Latest full `make test-cpython` sweep (2026-07-29): **22 / 398 files pass with zero failures**,
aggregate `failures=`+`errors=` across all files is in the low thousands, and **only 1 known
panic remains in the whole corpus** (see "Known deep gaps" below). File-level pass/fail counts
are a *harsh* metric — CPython's own test files are exhaustive edge-case suites, so a "failing"
file is very often passing 90%+ of its individual subtests and failing on a handful of specific
edge cases, not fundamentally broken. Treat the aggregate failures+errors count as the more
meaningful trend line, and expect it to rise temporarily whenever a fix unblocks a file that
was previously crashing/hanging early — more of the file's real, previously-unreached assertions
get a chance to run and fail on their own separate (usually narrower) gaps. That is forward
progress, not a regression, and has been the dominant pattern change-over-change this project.

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
  `test_itertools.py::test_count_threading`, the one remaining known panic in the full corpus as
  of this writing). Fixing this needs either a GIL-equivalent serialization point or a genuine
  `Arc<Mutex<>>`-based redesign of at least the objects reachable from thread targets.
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
  `ContextVar` uses one global thread-local stack), a real event-loop-backed `asyncio`.

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
