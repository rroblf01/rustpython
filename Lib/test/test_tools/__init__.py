"""Minimal stub of CPython's Lib/test/test_tools package.

Real CPython ships Tools/ scripts alongside the interpreter and this package
helps test them; this interpreter ships none, so `skip_if_missing` simply
never skips (a no-op decorator). Only the import contract matters to the
corpus (e.g. test.support.i18n_helper imports it and calls it as a plain
function statement inside assertMsgidsEqual)."""

import unittest


def skip_if_missing(tool=None, *, use_srcdir=True):
    """Return a no-op decorator; this interpreter has no Tools/ dir to gate."""
    return unittest.skipUnless(False, f"{tool} tool missing")


def skip_if_on_isolated_fs(func):
    return func
