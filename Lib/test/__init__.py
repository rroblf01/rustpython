# Dummy file to make this directory a package.
# Extend __path__ to include tests/cpython so `from test import test_genericpath`
# etc. works when running CPython's test suite directly (tests/cpython/test_ntpath.py
# does `from test import test_genericpath`). The cpython_runner harness executes
# each test file directly via the RustPython binary, not via `python -m test`.
# Without this, `import test.test_genericpath` fails because Lib/test has no
# test_genericpath.py while tests/cpython does (vendored CPython test suite).
import os as _os
try:
    __path__
except NameError:
    __path__ = []
_current = _os.path.dirname(__file__)
# Candidate locations for the vendored CPython test suite
_candidates = [
    _os.path.join(_os.path.dirname(_current), "tests", "cpython"),
    _os.path.join(_current, "..", "tests", "cpython"),
    _os.path.normpath(_os.path.join(_current, "../../tests/cpython")),
]
for _p in _candidates:
    _p = _os.path.normpath(_p)
    if _p not in __path__ and _os.path.isdir(_p):
        __path__.append(_p)
# Cleanup
del _os, _current, _candidates, _p
