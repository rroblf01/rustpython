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


# --- rare event counters (adaptive interpreter, 3.12+) -------------------
# CPython bumps one of these when it gives up specialising something (setting
# an instance's `__class__`, re-parenting a class, mutating the builtins dict
# or a function's `__code__`, installing an eval-frame hook). There is no
# adaptive interpreter here, but the OPERATIONS are still counted natively —
# `test_optimizer` asserts each one raises its counter by exactly one — and
# the table itself is read back from the `_rare_events` native module.
#
# The two tuning knobs below are read all over the corpus (93 files mention
# them); real values matter only to CPython's own specialiser, so any
# positive number behaves the same way here.
SPECIALIZATION_THRESHOLD = 1000
SPECIALIZATION_COOLDOWN = 0

try:
    from _rare_events import (
        get_rare_event_counters as _get_counters,
        reset_rare_event_counters as _reset_counters,
        set_eval_frame_default as _set_eval_frame_default,
        set_eval_frame_record as _set_eval_frame_record,
    )
except ImportError:  # pragma: no cover - the native module is always built
    def _get_counters():
        return {"set_class": 0, "set_bases": 0, "set_eval_frame_func": 0,
                "builtin_dict": 0, "func_modification": 0}

    def _reset_counters():
        pass

    def _set_eval_frame_record(records):
        pass

    def _set_eval_frame_default():
        pass


def get_rare_event_counters():
    return _get_counters()


def reset_rare_event_counters():
    _reset_counters()


def set_eval_frame_record(records):
    _set_eval_frame_record(records)


def set_eval_frame_default():
    _set_eval_frame_default()
