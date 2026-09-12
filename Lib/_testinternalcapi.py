"""Stub for CPython's _testinternalcapi (internal C API test hooks)."""
import sys
import unittest


def has_inline_values(obj):
    return False


def get_getpath_codeobject():
    raise unittest.SkipTest("getpath introspection not available")


def get_recursion_depth():
    """Return the interpreter's current Python-frame recursion depth.

    Real CPython's C extension reads `tstate->py_recursion_limit -
    tstate->py_recursion_remaining` directly; this pure-Python stub has no
    access to that counter, so it walks the frame chain instead (the same
    mechanism `Lib/test/support/__init__.py`'s own `get_recursion_depth()`
    falls back to when `_testinternalcapi` is unavailable). `sys._getframe(1)`
    (not `_getframe()`) skips THIS function's own frame so the count lines
    up with what that fallback path produces when called directly, since
    `test.support.get_recursion_depth()` applies the same `- 1` adjustment
    to whichever branch it took.
    """
    depth = 0
    frame = sys._getframe(1)
    while frame is not None:
        depth += 1
        frame = frame.f_back
    return depth
