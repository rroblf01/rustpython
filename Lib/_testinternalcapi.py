"""Stub for CPython's _testinternalcapi (internal C API test hooks)."""
import unittest


def has_inline_values(obj):
    return False


def get_getpath_codeobject():
    raise unittest.SkipTest("getpath introspection not available")
