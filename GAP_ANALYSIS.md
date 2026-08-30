# RustPython — CPython 3.14 Compatibility Gap Analysis

Last updated: August 30, 2026 (previous version dated August 29 — see `CLAUDE.md` for the current
architecture and the `cpython_test_suite_compat` memory topic for the full batch-by-batch history).

**2026-08-30 session**: fixed a real compiler bug found via a chain of investigation starting from
`doctest.py` never scanning a module's `__test__` dict (so any file registering doctests that way —
`test_descrtut`, `test_setcomps`, `test_unpack`, `test_generators`, ...  — silently ran ZERO of
them, "passing" only because nothing real ever executed). Running those doctests for real exposed
a genuine, previously-undetected compiler bug: `compile_function`/`compile_class_body` saved and
restored `self.labels`/`label_stack`/`loop_stack`/`pending_cleanup` when entering a nested
function/class's scope, but never `self.label_positions` — since `new_label()` allocates an id by
pushing to both vectors in lockstep, resetting one without the other let a nested `def`/`class`'s
own labels silently collide with, and overwrite, an id the ENCLOSING loop had already allocated.
Any loop containing a nested function/class definition whose own body also needed a label (an
inner loop, a comprehension, a lambda) could jump to a corrupted target on its next iteration —
observed as either re-running its own setup code every "iteration" or a live iterator ending up
where a callable was expected (`TypeError: 'range_iterator' object is not callable`). This is very
likely the real (or a major contributing) cause behind `test_generators`/`test_listcomps`'s
120s TIMEOUTs. Fixed by saving/restoring `label_positions` in the same lockstep as `labels`. A
second, related gap was fixed alongside it: the upfront cellvar/freevar analysis
(`analyze_function`) only walked literal `Stmt::FunctionDef`/`Stmt::ClassDef` nodes to find what a
nested scope needs relayed as a cell, missing a `lambda` reachable only through an expression
(`items = {(lambda: i) for i in range(5)}`) entirely, and separately never recognized a
comprehension's own `for x in ...` target as a real local of the enclosing function (this compiler
inlines comprehensions rather than giving them CPython's own separate scope). See the
`cpython_test_suite_compat` memory topic for the full chain and a note on `contextvars.Context`
(deferred, same calibre as the `property`-subclassing gap).

Same session, after the compiler fix: a further ~15-commit batch of smaller, individually-verified
fixes — `str`/`bytes.strip()`/`center()` bugs (wrong identity/wrong padding side), `utf-16`/`utf-32`
codecs (previously entirely unimplemented — silently fell back to raw UTF-8 bytes), `STORE_ATTR`/
`DELETE_ATTR` not walking the MRO for `__setattr__`/`__delattr__` (broke `unittest.mock`'s
`del`/set — then, once fixed, had to explicitly re-exclude `object`'s own default `__setattr__`/
`__delattr__` from that same MRO walk, since finding it was preempting `property` setters), a
native `__future__` module shadowing the real, complete vendored `Lib/__future__.py`, `exec(code,
dict)` never removing a name `del`eted during execution, several `csv` reader/writer
escapechar/quoting bugs, `base64`'s a85/b85/z85 encodings (didn't exist at all — added, verified
byte-for-byte against real CPython 3.14), `binascii`'s dozen `*2b_*`/`b2a_*` functions rejecting
`array.array`/`memoryview` (plus `crc_hqx`/`a2b_hex`/`b2a_hex` missing outright), a general parser
bug where bytes literals never handled `\a`/`\b`/`\f`/`\v` escapes (str literals did), and a real
infinite-hang bug in `PyObjectRef::str()`: any native string method (`encode()`, `.hex()`, ...)
called from inside a custom `__str__` re-entered `.str()` on the same object, which detected the
override again and called `__str__` again, forever — never touching the VM's frame stack, so it
hung instead of raising `RecursionError`. Fixed with a reentrancy guard mirroring the existing
`REPR_STACK`/`REPR_DEPTH` cycle detector in the same file (which only covered container types). A
narrower, deeper bug remains where the resulting *value* is still wrong for this exact pattern
(the object's backing value appears to get overwritten by its own first `__str__` result) — no
longer a hang, so lower priority; not attempted. All fixes verified via independent full
`make test-cpython` sweeps with zero net regressions each. Also identified (not attempted):
`collections.abc`'s ABC mixin methods (e.g. `MutableSequence.append()`) don't exist — both the
native `collections.abc` module and the vendored `Lib/_collections_abc.py` are far-reduced stubs
versus real CPython's ~1172-line module, which itself depends heavily on `abc.ABCMeta`/
`abstractmethod` machinery that has its own significant gaps (`test_abc.py`). A real fix needs
`abc.ABCMeta` solid first, then a vendor attempt of the real `_collections_abc.py` — same
"dedicated future session" calibre as `contextvars.Context` and `property` subclassing.

**2026-08-30 session, batch 2** (pace changed per user feedback: fast per-fix checks only, full
sweep deferred until several fixes land): four more fixes, three root-caused via parallel `Agent`
investigations. (1) `PyDict::apply_probed_set` bumped its iterator-invalidation version counter on
every `set()` call, including same-key value updates that don't change dict size and are legal
during iteration in real CPython — fixed 182 of 185 errors in `test_configparser.py` alone. (2)
`LOAD_ATTR`'s per-frame `attr_cache` is keyed by `(name, type)` with no per-instance component,
which is safe for methods/class attributes but not for values synthesized via native-backing
delegation (e.g. `.real`/`.imag` on a `complex`/`float` subclass, computed per-instance) — a second
same-type instance's lookup was returning the first instance's cached value; fixed by excluding
native-backing-delegated values from the cache. Likely affects any type with per-instance computed
attributes via this delegation path, not just complex/float. (3) `complex()`'s constructor combined
real/imaginary parts via plain float addition, which flushes `-0.0`'s sign under IEEE 754
(`0.0 + -0.0 == 0.0`); fixed to only add when genuinely needed, and added support for a complex
second argument (`complex(a+bj, c+dj)`) and a `native_backing_of` fallback for `__complex__`-less
instances. (4) The compiler's upfront closure analysis (`collect_nested_refs_inner`'s `ClassDef`
arm in `src/compiler/closure.rs`) filtered names referenced from *inside* a nested class's body
against the *enclosing* scope's own locals before deciding they needed relaying as a cell — but
class bodies are always their own code object using `LOAD_CLASSDEREF`/`LOAD_DEREF`, so a name being
locally resolvable one scope up doesn't mean no relay is needed. Root cause of `test_abc.py`'s
`test_descriptors_with_abstractmethod` `NameError: name 'D' is not defined` (a method nested in a
class nested in a function building `class C(metaclass=meta): ...` then a sibling `class D(C):
...`). Fixed by dropping that filter for body-referenced names (kept for header
bases/keywords/decorators, which genuinely are evaluated directly in the enclosing frame). All four
verified via `cargo test` + `make test-python` + targeted file runs + `git stash`-based A/B across
closure-sensitive files (test_scope/test_class/test_descr/test_generators/test_listcomps/
test_dictcomps/test_setcomps/test_functools/test_decorators/test_super/test_metaclass); full sweep
pending per the batching change. See `cpython_test_suite_compat` memory topic, parte 7, for detail.

## How "compatibility" is actually measured here

There is no single trustworthy percentage for "how done is this interpreter." What exists is:

- **`make test-cpython`**: runs all 398 real CPython 3.14 stdlib test files (vendored verbatim
  in `tests/cpython/`, see `tests/cpython/README.md`) against a release build. This is the
  primary, most objective signal.
- **`make test-python`**: a small hand-written regression suite (`tests/*.py`), currently
  **18 passed / 16 failed** — kept stable as a smoke-test baseline every change must not regress.

Latest full `make test-cpython` sweep (2026-08-30): **98 / 398 files pass with zero failures**
(296 FAIL, 4 TIMEOUT) — down 2 from the August 29 count of 100, but not a regression: the 2 lost
files (`test_setcomps`, `test_unpack`) were only ever "passing" because their real doctest content
silently never ran (the `__test__` dict bug above) — now that it does, each has exactly one
remaining narrow gap (a genuine comprehension-scope-isolation gap for `test_setcomps`; an artifact
of running the file standalone rather than as `test.test_unpack` — confirmed real CPython 3.14
fails that exact same doctest run the same way — for `test_unpack`). TIMEOUT dropped from 5 to 4
thanks to the `label_positions` fix. File-level pass/fail counts are a *harsh*
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
order (see the SipHash work in git history) hitting different code paths run to run. Treat a
TIMEOUT or panic on either of these two files as inconclusive on its own; rerun before attributing
it to a change. (2026-08-30: `test_listcomps`'s TIMEOUTs at least partly had a real, DETERMINISTIC
cause — the same `label_positions` compiler bug described above, since it contains the identical
conjoin/Queens/Knights backtracking-solver doctest pattern as `test_generators`, fixed by the same
commit — but the file may still have a separate, genuinely non-deterministic failure mode on top
of that; not fully disentangled.)

**LTO footgun, found via `test_pprint`/`test_difflib` (fixed):** these two intermittently failed
in a way that first looked like parallel-sweep flakiness but turned out to be a real, deterministic
`cargo build` (debug) vs `cargo build --release` (this project's `lto = "thin"` profile) behavior
difference. Several native functions meant to have *distinct* per-type identities (the
`native_repr_fn!`-generated `__repr__`s, so `pprint`'s `type(obj).__repr__`-keyed dispatch can tell
types apart) had byte-identical compiled bodies, which LLVM's MergeFunctions/identical-code-folding
pass legally re-merges into one address under LTO — silently breaking any code relying on those
functions staying distinct, in the `release` build ONLY (`make test-cpython` always builds
release). Fixed by forcing a distinct per-function constant via `std::hint::black_box`; see the
comment on `native_repr_fn!` in `src/object/builtins.rs`. **General lesson**: any group of native
Rust functions meant to be distinguishable by pointer identity (`fn_addr_eq`, used as dict keys,
etc.) is at risk if their bodies could plausibly compile identically — verify a fix like this
against BOTH `cargo build` and `cargo build --release` before trusting it, not just one.

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
