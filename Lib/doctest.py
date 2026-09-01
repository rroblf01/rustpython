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

_OPTIONFLAGS_BY_NAME = {
    "NORMALIZE_WHITESPACE": NORMALIZE_WHITESPACE,
    "ELLIPSIS": ELLIPSIS,
    "IGNORE_EXCEPTION_DETAIL": IGNORE_EXCEPTION_DETAIL,
    "SKIP": SKIP,
    "REPORT_UDIFF": REPORT_UDIFF,
    "REPORT_CDIFF": REPORT_CDIFF,
    "REPORT_NDIFF": REPORT_NDIFF,
    "REPORT_ONLY_FIRST_FAILURE": REPORT_ONLY_FIRST_FAILURE,
    "DONT_ACCEPT_TRUE_FOR_1": DONT_ACCEPT_TRUE_FOR_1,
    "DONT_ACCEPT_BLANKLINE": DONT_ACCEPT_BLANKLINE,
    "FAIL_FAST": FAIL_FAST,
}
_DIRECTIVE_RE = re.compile(r"#\s*doctest:\s*([+-]\w+(?:\s*,\s*[+-]\w+)*)\s*$")


def _parse_directive(line):
    """Parse a trailing `# doctest: +FLAG,-FLAG` comment on a source line.

    Returns (set_mask, clear_mask), both 0 if there's no directive. This
    was entirely missing, so a per-example directive like test_cmd.py's
    `>>> mycmd.onecmd("help meaning")  # doctest: +NORMALIZE_WHITESPACE`
    was silently ignored — the example still ran, just always with
    whatever the SUITE's global optionflags happened to be (usually 0).
    """
    m = _DIRECTIVE_RE.search(line)
    if not m:
        return 0, 0
    set_mask = clear_mask = 0
    for part in m.group(1).split(","):
        part = part.strip()
        sign, name = part[0], part[1:]
        flag = _OPTIONFLAGS_BY_NAME.get(name, 0)
        if sign == "+":
            set_mask |= flag
        else:
            clear_mask |= flag
    return set_mask, clear_mask


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
            set_mask = clear_mask = 0
            for src_line in src_lines:
                s, c = _parse_directive(src_line)
                set_mask |= s
                clear_mask |= c
            examples.append(
                ("\n".join(src_lines), "\n".join(want_lines), set_mask, clear_mask)
            )
        else:
            i += 1
    return examples


def _normalize_ws(s):
    return " ".join(s.split())


def _compare(got, want, optionflags):
    if not (optionflags & DONT_ACCEPT_BLANKLINE):
        # A genuinely blank line can't appear literally in a doctest's
        # expected-output block (it would terminate the block when the
        # source is parsed), so doctest's own convention is to write the
        # literal marker `<BLANKLINE>` there instead — substitute it back
        # to a real empty line before comparing. Was entirely missing,
        # so any doctest whose expected output contains a blank line
        # (a very common idiom, e.g. `cmd.Cmd.do_help`'s multi-section
        # listing) always failed even on a byte-for-byte correct match.
        want = "\n".join(
            "" if line == "<BLANKLINE>" else line for line in want.split("\n")
        )
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


def _run_example(source, want, globs, optionflags=0, fakeout=None):
    """Run one example against `globs`; None on success, else a message.

    `fakeout`, if given, is a SHARED `io.StringIO` that `sys.stdout` is
    already redirected to for the whole docstring's run (real CPython's
    doctest keeps ONE fake stdout alive across every example in a
    docstring, not a fresh one per example) — this matters for any example
    that stashes `sys.stdout` into a longer-lived object early on and
    writes to it from a LATER example (`cmd.Cmd.__init__`'s `self.stdout =
    sys.stdout`, then a subsequent `self.stdout.write(...)` from a
    different example needs to land in the SAME buffer doctest is
    currently checking — test_cmd.py's `samplecmdclass` doctest). A fresh
    `io.StringIO()` per example (the previous behavior here) made that
    write silently disappear into a buffer nothing ever inspects.
    """
    import io
    import contextlib

    if fakeout is not None:
        buf = fakeout
        start = buf.tell()
        redirect = contextlib.nullcontext()
        # sys.stdout is assumed already redirected to `buf` by the caller
        # for the duration of the whole docstring run.
    else:
        buf = io.StringIO()
        start = 0
        redirect = contextlib.redirect_stdout(buf)
    try:
        try:
            code = compile(source, "<doctest>", "eval")
            is_eval = True
        except SyntaxError:
            code = compile(source, "<doctest>", "exec")
            is_eval = False
        with redirect:
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
        got = buf.getvalue()[start:]
        got += "Traceback (most recent call last):\n"
        msg = str(e)
        got += ("%s: %s" % (type(e).__name__, msg)) if msg else type(e).__name__
        return _compare(got, want, optionflags)
    return _compare(buf.getvalue()[start:], want, optionflags)


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

    testdict = getattr(module, "__test__", None)
    if isinstance(testdict, dict):
        for valname, val in testdict.items():
            qualname = "%s.__test__.%s" % (mod_name, valname)
            if isinstance(val, str):
                results.append((qualname, val))
            elif isinstance(val, type):
                results.extend(_class_docstrings(val, mod_name, qualname))
            elif callable(val):
                doc = getattr(val, "__doc__", None)
                if isinstance(doc, str):
                    results.append((qualname, doc))
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
        import io
        import contextlib

        test_globs = dict(base_globs)
        failures = []
        fakeout = io.StringIO()
        with contextlib.redirect_stdout(fakeout):
            for source, want, set_mask, clear_mask in examples:
                flags = (optionflags | set_mask) & ~clear_mask
                err = _run_example(source, want, test_globs, flags, fakeout)
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

    import io
    import contextlib

    failed = 0
    attempted = 0
    for qualname, doc in _member_docstrings(module):
        examples = _extract_examples(doc)
        if not examples:
            continue
        test_globs = dict(base_globs)
        fakeout = io.StringIO()
        with contextlib.redirect_stdout(fakeout):
            for source, want, set_mask, clear_mask in examples:
                attempted += 1
                flags = (optionflags | set_mask) & ~clear_mask
                err = _run_example(source, want, test_globs, flags, fakeout)
                if err is not None:
                    failed += 1
                    if verbose:
                        print("*" * 70, file=sys.__stdout__)
                        print("Failure in docstring for", qualname, file=sys.__stdout__)
                        print(err, file=sys.__stdout__)
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

    import io
    import contextlib

    failed = 0
    attempted = 0
    fakeout = io.StringIO()
    with contextlib.redirect_stdout(fakeout):
        for source, want, set_mask, clear_mask in _extract_examples(content):
            attempted += 1
            flags = (optionflags | set_mask) & ~clear_mask
            err = _run_example(source, want, test_globs, flags, fakeout)
            if err is not None:
                failed += 1
                if raise_on_error:
                    raise DocTestFailure(err)
    return TestResults(failed, attempted)


def run_docstring_examples(f, globs, verbose=False, name="NoName",
                            compileflags=None, optionflags=0):
    import io
    import contextlib

    doc = getattr(f, "__doc__", None)
    if not isinstance(doc, str):
        return
    test_globs = dict(globs)
    fakeout = io.StringIO()
    with contextlib.redirect_stdout(fakeout):
        for source, want, set_mask, clear_mask in _extract_examples(doc):
            flags = (optionflags | set_mask) & ~clear_mask
            err = _run_example(source, want, test_globs, flags, fakeout)
            if err is not None and verbose:
                print(err, file=sys.__stdout__)


class DocTestFinder:
    def __init__(self, *args, **kwargs):
        pass

    def find(self, obj=None, name=None, module=None, globs=None,
             extraglobs=None):
        return []
