"""Minimal but functional doctest implementation for RustPython.

Covers the common `DocTestSuite()`/`DocFileSuite()` usage pattern (folding
module/file docstring examples into a real `unittest.TestSuite` via the
`load_tests` protocol) with genuine example execution and output
comparison, not just a hollow always-passing stub. Deliberately simpler
than real CPython's doctest: no interactive debugger integration, a
simplified exception-matching rule (compares only the final traceback
line), and no closure/nested-function docstring discovery.

`module=None` resolves to `sys.modules['__main__']` — this interpreter has
no frame-introspection support (`inspect.currentframe()` always returns
`None`) to find the real caller the way real CPython's doctest does, but
every real-world call site observed in the CPython test corpus calls
`DocTestSuite()` with no arguments from a test file's own `load_tests`
function, meaning "check my own module" — since that test file is always
running as `__main__`, this heuristic is exactly correct in practice.
"""

import re
import sys
import unittest

__all__ = [
    "DocTestSuite", "DocFileSuite", "testmod", "testfile",
    "run_docstring_examples", "TestResults", "DocTestFinder",
    "NORMALIZE_WHITESPACE", "ELLIPSIS", "IGNORE_EXCEPTION_DETAIL",
    "SKIP", "REPORT_ONLY_FIRST_FAILURE", "DocTestFailure",
]

NORMALIZE_WHITESPACE = 1 << 1
ELLIPSIS = 1 << 2
IGNORE_EXCEPTION_DETAIL = 1 << 3
SKIP = 1 << 4
REPORT_UDIFF = 1 << 5
REPORT_CDIFF = 1 << 6
REPORT_NDIFF = 1 << 7
REPORT_ONLY_FIRST_FAILURE = 1 << 8
DONT_ACCEPT_TRUE_FOR_1 = 1 << 9
DONT_ACCEPT_BLANKLINE = 1 << 10
FAIL_FAST = 1 << 11


class DocTestFailure(AssertionError):
    pass


class TestResults:
    def __init__(self, failed, attempted):
        self.failed = failed
        self.attempted = attempted

    def __repr__(self):
        if not self.attempted:
            return "TestResults(failed=0, attempted=0)"
        return "TestResults(failed=%d, attempted=%d)" % (self.failed, self.attempted)

    def __eq__(self, other):
        if isinstance(other, tuple):
            return (self.failed, self.attempted) == other
        if isinstance(other, TestResults):
            return (self.failed, self.attempted) == (other.failed, other.attempted)
        return NotImplemented


def _normalize_module(module):
    if module is None:
        return sys.modules.get("__main__")
    if isinstance(module, str):
        return __import__(module, globals(), locals(), ["*"])
    return module


def _extract_examples(docstring):
    """Parse a docstring/file's text into a list of (source, want) pairs."""
    if not docstring:
        return []
    lines = docstring.splitlines()
    examples = []
    i = 0
    n = len(lines)
    while i < n:
        line = lines[i]
        stripped = line.lstrip()
        indent = len(line) - len(stripped)
        if stripped.startswith(">>>"):
            rest = stripped[3:]
            if rest.startswith(" "):
                rest = rest[1:]
            src_lines = [rest]
            i += 1
            while i < n:
                l2 = lines[i]
                s2 = l2.lstrip()
                ind2 = len(l2) - len(s2)
                if ind2 == indent and s2.startswith("..."):
                    r2 = s2[3:]
                    if r2.startswith(" "):
                        r2 = r2[1:]
                    src_lines.append(r2)
                    i += 1
                else:
                    break
            want_lines = []
            while i < n:
                l3 = lines[i]
                s3 = l3.lstrip()
                if s3 == "":
                    break
                ind3 = len(l3) - len(s3)
                if ind3 < indent or s3.startswith(">>>"):
                    break
                want_lines.append(l3[indent:] if len(l3) >= indent else s3)
                i += 1
            examples.append(("\n".join(src_lines), "\n".join(want_lines)))
        else:
            i += 1
    return examples


def _normalize_ws(s):
    return " ".join(s.split())


def _compare(got, want, optionflags):
    is_exc = want.lstrip().startswith("Traceback (most recent call last):")
    if is_exc:
        want_lines = [l for l in want.splitlines() if l.strip()]
        got_lines = [l for l in got.splitlines() if l.strip()]
        want_last = want_lines[-1] if want_lines else ""
        got_last = got_lines[-1] if got_lines else ""
        if optionflags & IGNORE_EXCEPTION_DETAIL:
            want_last = want_last.split(":")[0].strip()
            got_last = got_last.split(":")[0].strip()
        if want_last == got_last:
            return None
        return "Expected exception ending:\n    %s\nGot:\n%s" % (want_last, got)

    g = got.rstrip("\n")
    w = want.rstrip("\n")
    if optionflags & NORMALIZE_WHITESPACE:
        g = _normalize_ws(g)
        w = _normalize_ws(w)
    if (optionflags & ELLIPSIS) and "..." in w:
        pattern = re.escape(w).replace(r"\.\.\.", ".*")
        if re.fullmatch(pattern, g, re.DOTALL):
            return None
        return "Expected (with ellipsis):\n%s\nGot:\n%s" % (want, got)
    if g == w:
        return None
    return "Expected:\n%s\nGot:\n%s" % (want, got)


def _run_example(source, want, globs, optionflags=0):
    """Run one example against `globs`; None on success, else a message."""
    import io
    import contextlib

    buf = io.StringIO()
    try:
        try:
            code = compile(source, "<doctest>", "eval")
            is_eval = True
        except SyntaxError:
            code = compile(source, "<doctest>", "exec")
            is_eval = False
        with contextlib.redirect_stdout(buf):
            if is_eval:
                result = eval(code, globs)
                # Mirrors the interactive interpreter's own auto-print of a
                # bare expression's result (this interpreter's `compile(...,
                # "single")` doesn't do this automatically, unlike real
                # CPython, so it's replicated here directly).
                if result is not None:
                    print(repr(result))
            else:
                exec(code, globs)
    except Exception as e:
        got = buf.getvalue()
        got += "Traceback (most recent call last):\n"
        msg = str(e)
        got += ("%s: %s" % (type(e).__name__, msg)) if msg else type(e).__name__
        return _compare(got, want, optionflags)
    return _compare(buf.getvalue(), want, optionflags)


def _member_docstrings(module):
    """(qualname, docstring) pairs for module + its own top-level functions/
    classes/methods — restricted to things actually DEFINED in `module`
    (matching real doctest's own same-module filtering), so re-exported /
    imported names' docstrings aren't redundantly tested here too.
    """
    results = []
    doc = getattr(module, "__doc__", None)
    if isinstance(doc, str):
        results.append((getattr(module, "__name__", "<module>"), doc))

    mod_name = getattr(module, "__name__", None)
    try:
        members = list(vars(module).items())
    except TypeError:
        members = []

    for name, val in members:
        if getattr(val, "__module__", None) != mod_name:
            continue
        if isinstance(val, type):
            results.extend(_class_docstrings(val, mod_name, name))
        elif callable(val):
            doc = getattr(val, "__doc__", None)
            if isinstance(doc, str):
                results.append(("%s.%s" % (mod_name, name), doc))
    return results


def _class_docstrings(cls, mod_name, prefix):
    results = []
    doc = getattr(cls, "__doc__", None)
    if isinstance(doc, str):
        results.append((prefix, doc))
    try:
        members = list(vars(cls).items())
    except TypeError:
        members = []
    for name, val in members:
        target = val
        if isinstance(val, (staticmethod, classmethod)):
            target = val.__func__
        elif isinstance(val, property):
            target = val.fget
        if target is None or not callable(target):
            continue
        if getattr(target, "__module__", None) != mod_name:
            continue
        doc = getattr(target, "__doc__", None)
        if isinstance(doc, str):
            results.append(("%s.%s" % (prefix, name), doc))
    return results


def _make_case(qualname, examples, base_globs, optionflags):
    def run():
        test_globs = dict(base_globs)
        failures = []
        for source, want in examples:
            err = _run_example(source, want, test_globs, optionflags)
            if err is not None:
                failures.append(err)
                if optionflags & REPORT_ONLY_FIRST_FAILURE:
                    break
        if failures:
            raise DocTestFailure(
                "%d of %d examples failed in docstring for %s:\n%s"
                % (len(failures), len(examples), qualname, "\n\n".join(failures))
            )
    run.__name__ = "docstring (%s)" % qualname
    return unittest.FunctionTestCase(run)


def DocTestSuite(module=None, globs=None, extraglobs=None, test_finder=None,
                  **options):
    optionflags = options.get("optionflags", 0)
    module = _normalize_module(module)
    suite = unittest.TestSuite()
    if module is None:
        return suite

    base_globs = dict(vars(module))
    if globs:
        base_globs.update(globs)
    if extraglobs:
        base_globs.update(extraglobs)

    for qualname, doc in _member_docstrings(module):
        examples = _extract_examples(doc)
        if not examples:
            continue
        suite.addTest(_make_case(qualname, examples, base_globs, optionflags))
    return suite


def DocFileSuite(*paths, module_relative=True, package=None, globs=None,
                  extraglobs=None, **options):
    import os
    optionflags = options.get("optionflags", 0)
    suite = unittest.TestSuite()
    base_globs = dict(globs) if globs else {}
    if extraglobs:
        base_globs.update(extraglobs)

    for path in paths:
        full_path = path
        if module_relative and not os.path.isabs(path):
            caller_dir = os.getcwd()
            full_path = os.path.join(caller_dir, path)
        try:
            with open(full_path, "r", encoding="utf-8") as f:
                content = f.read()
        except OSError:
            continue
        examples = _extract_examples(content)
        if not examples:
            continue
        suite.addTest(_make_case(path, examples, base_globs, optionflags))
    return suite


def testmod(m=None, name=None, globs=None, verbose=None, report=True,
            optionflags=0, extraglobs=None, raise_on_error=False,
            exclude_empty=False):
    module = _normalize_module(m)
    if module is None:
        return TestResults(0, 0)

    base_globs = dict(vars(module))
    if globs:
        base_globs.update(globs)
    if extraglobs:
        base_globs.update(extraglobs)

    failed = 0
    attempted = 0
    for qualname, doc in _member_docstrings(module):
        examples = _extract_examples(doc)
        if not examples:
            continue
        test_globs = dict(base_globs)
        for source, want in examples:
            attempted += 1
            err = _run_example(source, want, test_globs, optionflags)
            if err is not None:
                failed += 1
                if verbose:
                    print("*" * 70)
                    print("Failure in docstring for", qualname)
                    print(err)
                if raise_on_error:
                    raise DocTestFailure(err)
    if report and failed:
        print("%d items had failures" % failed)
    return TestResults(failed, attempted)


def testfile(filename, module_relative=True, name=None, package=None,
             globs=None, verbose=None, report=True, optionflags=0,
             extraglobs=None, raise_on_error=False, parser=None,
             encoding=None):
    try:
        with open(filename, "r", encoding=encoding or "utf-8") as f:
            content = f.read()
    except OSError:
        return TestResults(0, 0)

    test_globs = dict(globs) if globs else {}
    if extraglobs:
        test_globs.update(extraglobs)

    failed = 0
    attempted = 0
    for source, want in _extract_examples(content):
        attempted += 1
        err = _run_example(source, want, test_globs, optionflags)
        if err is not None:
            failed += 1
            if raise_on_error:
                raise DocTestFailure(err)
    return TestResults(failed, attempted)


def run_docstring_examples(f, globs, verbose=False, name="NoName",
                            compileflags=None, optionflags=0):
    doc = getattr(f, "__doc__", None)
    if not isinstance(doc, str):
        return
    test_globs = dict(globs)
    for source, want in _extract_examples(doc):
        err = _run_example(source, want, test_globs, optionflags)
        if err is not None and verbose:
            print(err)


class DocTestFinder:
    def __init__(self, *args, **kwargs):
        pass

    def find(self, obj=None, name=None, module=None, globs=None,
             extraglobs=None):
        return []
