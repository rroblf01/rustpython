"""Tool for measuring execution time of small code snippets.

Library usage: see the Timer class.
Command line usage:
    python timeit.py [-n N | --number=N] [-r N | --repeat=N] [-s S |
    --setup=S] [-u | --unit=UNIT] [-v | --verbose] [--] [statement]

Options:
  -n/--number/--number=N: how many times to execute 'statement'
  -r/--repeat/--repeat=N: how many times to repeat the timer (default 5)
  -s/--setup/--setup=S: statement to be executed once initially
                        (default pass). Execution time of this setup
                        statement is NOT timed.
  -p/--process: use time.process_time() instead of default timer
  -v/--verbose: print raw timing results; repeat for more digits precision
  -u/--unit=U: use nsec, usec, msec, or sec for output unit (default auto)
  -h/--help: print this usage message and exit
  --: end options; statement to be timed

Statement to be timed may be given as a string or as a callable object.
"""

import sys as _sys
import time as _time
import itertools as _itertools

import _timeit as _native

_NativeTimer = getattr(_native, 'Timer', object)

from _timeit import (           # noqa: F401 -- re-exported
    default_number as default_number_native,
    default_repeat as default_repeat_native,
)

__all__ = ["Timer", "timeit", "repeat", "default_timer",
           "default_number", "default_repeat"]

default_number = 1000000
default_repeat = 5
default_timer = _time.perf_counter


def reindent(src, indent):
    """Add 'indent' spaces of extra indentation to every line AFTER the
    first (CPython semantics: blanks included, first line untouched)."""
    if indent == 0:
        return src
    pad = " " * indent
    lines = src.split("\n")
    return "\n".join([lines[0]] + [pad + l for l in lines[1:]])


def _template_func(setup, func):
    return func or "pass"


_NativeTimerBase = getattr(_native, "Timer", object)




_INVALID_TIMEIT_FIRST = ("return", "yield", "break", "continue")

def _reject_invalid_timeit_src(src, what):
    stripped = src.lstrip()
    toks = stripped.replace(",", " , ").replace(";", " ; ").split()
    if len(toks) >= 3 and toks[0] == "from" and toks[-1] == "*" and toks[-2] == "import":
        raise SyntaxError(
            "%s statement must not contain a wildcard import" % what
        )
    first_word = toks[0] if toks else ""
    if False and first_word.startswith("from ") and first_word.endswith("*"):
        # 'from timeit import *' etc. are valid Python but invalid timeit
        raise SyntaxError(
            "%s statement must not contain a wildcard import" % what
        )
    if first_word in _INVALID_TIMEIT_FIRST:
        raise SyntaxError("%s statement contains %r" % (what, first_word))

class Timer(_NativeTimerBase):
    """Class for timing execution speed of small code snippets."""

    def __init__(self, stmt="pass", setup="pass", timer=default_timer,
                 globals=None):
        if stmt is None or setup is None:
            raise ValueError("stmt/setup expression must be a str or callable")
        # Drop whitespace-only lines and dedent; normalize blank body to pass.
        if isinstance(stmt, str):
            stmt = "\n".join(l for l in stmt.split("\n") if l.strip())
            if not stmt.strip():
                stmt = "pass"
        if isinstance(setup, str):
            setup = "\n".join(l for l in setup.split("\n") if l.strip()) or "pass"
        if isinstance(stmt, str):
            _reject_invalid_timeit_src(stmt, "statement")
            compile(stmt, "<timeit-stmt>", "exec")
        if isinstance(setup, str):
            _reject_invalid_timeit_src(setup, "setup")
            compile(setup, "<timeit-setup>", "exec")
        self._timer_fn = timer if callable(timer) else default_timer
        self._globals_v = _safe_dict(globals) if globals else {}
        self._stmt_v = stmt
        self._setup_v = setup
        _NativeTimer.__init__(self, stmt, setup, timer,
                              globals=self._globals_v or None)

    def print_exc(self, file=None):
        """Print the traceback of the currently-handled exception, without
        the chained-context noise (matches CPython Timer.print_exc)."""
        import traceback
        if file is None:
            file = _sys.stderr
        info = _sys.exc_info()
        if info[0] is None:
            return
        exc = info[1]
        saved_ctx = getattr(exc, '__context__', None)
        try:
            try:
                exc.__context__ = None
            except Exception:
                pass
            traceback.print_exception(info[0], exc, info[2], file=file)
        finally:
            try:
                exc.__context__ = saved_ctx
            except Exception:
                pass

    # alias used by some callers


def _exec_stmt(code, g):
    _native._run_in_globals(code, _pydict(g))


def _safe_dict(d):
    """Convert a dict-like object (including PyObject::Globals views) to a
    plain dict, avoiding internal .keys() calls that may fail on views."""
    if d is None:
        return {}
    out = {}
    try:
        for k in d:
            out[k] = d[k]
    except TypeError:
        pass
    return out


def timeit(stmt="pass", setup="pass", timer=default_timer,
           number=default_number, globals=None):
    g = _safe_dict(globals) if globals else {}
    t = Timer(stmt=stmt, setup=setup, timer=timer, globals=g)
    return t.timeit(number)


def repeat(stmt="pass", setup="pass", timer=default_timer,
           repeat=default_repeat, number=default_number, globals=None):
    g = _safe_dict(globals) if globals else {}
    t = Timer(stmt=stmt, setup=setup, timer=timer, globals=g)
    return t.repeat(repeat, number)


# ── CLI ──────────────────────────────────────────────────────────────

_units = ["sec", "msec", "usec", "nsec"]
_scales = [1.0, 1e-3, 1e-6, 1e-9]


def _format_time(secs):
    """Format seconds choosing the largest unit giving a value >= 1."""
    secs = float(secs)
    unit = _units[-1]
    val = secs / _scales[-1]
    for u, sc in zip(_units, _scales):
        if secs / sc >= 1.0:
            unit = u
            val = secs / sc
            break
    return "%g %s" % (round(val, 3), unit)


def main(args=None, *, _wrap_timer=None):
    """CLI entry point (mirrors CPython's timeit.main)."""
    if args is None:
        args = _sys.argv[1:]
    opts_number = None
    opts_repeat = default_repeat
    setups = []
    unit = None
    verbose = False
    stmt_parts = []
    arglist = list(args)
    i = 0
    while i < len(arglist):
        a = arglist[i]
        if a == "-h" or a == "--help":
            # Exact bytes: tests capture stdout and compare with __doc__.
            _sys.stdout.write(__doc__)
            return
        elif a.startswith("--number="):
            opts_number = int(a.split("=", 1)[1])
        elif a == "-n":
            i += 1
            opts_number = int(arglist[i])
        elif a.startswith("-n") and len(a) > 2 and a[2:].lstrip("-").isdigit():
            opts_number = int(a[2:])
        elif a.startswith("--repeat="):
            opts_repeat = int(a.split("=", 1)[1])
        elif a == "-r":
            i += 1
            opts_repeat = max(1, int(arglist[i]))
        elif a.startswith("-r") and len(a) > 2 and a[2:].lstrip("-").isdigit():
            opts_repeat = max(1, int(a[2:]))
        elif a.startswith("--setup="):
            setups.append(a.split("=", 1)[1])
        elif a == "-s":
            i += 1
            setups.append(arglist[i])
        elif a.startswith("--unit="):
            unit = a.split("=", 1)[1]
        elif a == "-u":
            i += 1
            unit = arglist[i]
        elif a == "-vv" or a == "--verbose" and verbose:
            verbose = 2
        elif a == "-vv" or (a.startswith("-v") and a.count("v") > 1):
            verbose = 2
        elif a == "-v" or a == "--verbose":
            verbose = max(1, verbose if isinstance(verbose, int) else 0)
        elif a == "-p" or a == "--process":
            pass
        elif a == "--":
            pass
        elif a.startswith("-") and a != "-":
            print("option %s not recognized" % a)
            print("use -h/--help for command line help")
            return
        else:
            stmt_parts.append(a)
        i += 1

    stmt = "\n".join(stmt_parts) if stmt_parts else None

    setup_src = "\n".join(setups) if setups else "pass"
    timer_fn = default_timer
    if _wrap_timer is not None:
        timer_fn = _wrap_timer(timer_fn)
    t = Timer(stmt=stmt or "pass", setup=setup_src, timer=timer_fn)

    try:
        if opts_number is None:
            def _cb(n, t_taken):
                if verbose >= 1:
                    print("%d loop%s -> %g sec" % (
                        n, "s" if n != 1 else "", t_taken
                    ))
            num_loops, _t = t.autorange(_cb if verbose else None)
        else:
            num_loops = opts_number
        results = t.repeat(opts_repeat, num_loops)
    except:
        t.print_exc()
        return
    best = min(results)
    if verbose >= 1:
        print()
        raw = ", ".join(_format_time(r) for r in results) if isinstance(results, list) else ""
        print("raw times: %s" % raw)
        print()
    per_loop = best / num_loops if num_loops else best
    if unit is not None:
        valid_units = {"nsec": 1e-9, "usec": 1e-6, "msec": 1e-3, "sec": 1.0}
        if unit not in valid_units:
            _sys.stderr.write(
                "Unrecognized unit. Please select nsec, usec, msec, or sec.\n"
            )
            return
        scale = valid_units[unit]
        printed = "%.3g %s" % (per_loop / scale, unit)
    else:
        printed = _format_time(per_loop)

    loops_word = "loop" if num_loops == 1 else "loops"
    print("%d %s, best of %d: %s per loop"
          % (num_loops, loops_word, opts_repeat, printed))


if __name__ == "__main__":
    main()
