# CPython test suite

The 398 `test_*.py` files from CPython's own `Lib/test/` (branch `3.14`,
commit `41904952f5958d0f895f79a6b4f3d20cf6f9fbfd`), vendored verbatim as an
external compatibility corpus for this interpreter.

Not wired into `make test-python` (which only globs `tests/*.py`, not
subdirectories) — most of these still fail or need parts of `test.support`
not yet vendored (see `Lib/test/support/`). Run one directly against the
debug build to check a specific file:

```
./target/debug/rustpython tests/cpython/test_foo.py
```

See the `cpython_test_suite_compat` memory entry (or ask Claude) for the
current status of which files pass, which general interpreter bugs were
found/fixed by running this corpus, and what's still open.
