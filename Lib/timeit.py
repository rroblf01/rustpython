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
    """Helper to reindent a multi-line statement."""
    if indent == 0:
        return src
    pad = " " * indent
    return "\n".join(
        line if not line.strip() else pad + line for line in src.split("\n")
    )


def _template_func(setup, func):
    return func or "pass"


_NativeTimerBase = getattr(_native, "Timer", object)


class Timer(_NativeTimerBase):
    """Class for timing execution speed of small code snippets."""

    def __init__(self, stmt="pass", setup="pass", timer=default_timer,
                 globals=None):
        if stmt is None:
            raise ValueError("stmt expression must be a str or callable")
        if isinstance(stmt, str):
            compile(stmt, "<timeit-stmt>", "exec")   # SyntaxError on bad code
        if isinstance(setup, str) and setup.strip():
            compile(setup, "<timeit-setup>", "exec")
        self._stmt_v = stmt
        self._setup_v = setup
        self._timer_fn = timer if callable(timer) else default_timer
        self._globals_v = dict(globals) if globals else {}
        # native base does the heavy lifting (compiles str bodies, honors an
        # injected timer callable via _timer attribute)
        _NativeTimer.__init__(self, stmt if not isinstance(stmt, str) else stmt,
                              setup if isinstance(setup, str) else "pass",
                              timer or None)

    def print_exc(self, file=None):
        """Convenience to print the traceback of the last failed run."""
        import traceback
        if file is None:
            file = _sys.stderr
        traceback.print_exc(file=file)

    # alias used by some callers


def _exec_stmt(code, g):
    _native._run_in_globals(code, _pydict(g))


def _pydict(d):
    """Convert our dict to a plain dict usable by the bridge."""
    out = {}
    for k, v in d.items():
        out[k] = v
    return out


def timeit(stmt="pass", setup="pass", timer=default_timer,
           number=default_number, globals=None):
    g = dict(globals) if globals else {}
    t = Timer(stmt=stmt, setup=setup, timer=timer, globals=g)
    return t.timeit(number)


def repeat(stmt="pass", setup="pass", timer=default_timer,
           repeat=default_repeat, number=default_number, globals=None):
    g = dict(globals) if globals else {}
    t = Timer(stmt=stmt, setup=setup, timer=timer, globals=g)
    return t.repeat(repeat, number)


# ── CLI ──────────────────────────────────────────────────────────────

_units = ["nsec", "usec", "msec", "sec"]
_scales = [1e-9, 1e-6, 1e-3, 1.0]


def _format_time(usecs):
    usecs = float(usecs)
    for unit, scale in zip(_units[::-1], _scales[::-1]):
        if usecs / scale >= 1.0:
            break
    return "%g %s" % (round(usecs / scale, 3), unit)


def main(args=None, *, _wrap_timer=None):
    """CLI entry point (mirrors CPython's timeit.main)."""
    if args is None:
        args = _sys.argv[1:]
    opts_number = None
    opts_repeat = default_repeat
    setups = []
    unit = None
    verbose = False
    stmt = None
    arglist = list(args)
    i = 0
    while i < len(arglist):
        a = arglist[i]
        if a == "-h" or a == "--help":
            print(__doc__)
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
            opts_repeat = int(arglist[i])
        elif a.startswith("-r") and len(a) > 2 and                 arglist[i][2:].lstrip("-").isdigit():
            opts_repeat = int(a[2:])
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
        elif a == "-v" or a == "--verbose":
            verbose = True
        elif a == "-p" or a == "--process":
            pass
        elif a == "--":
            pass
        elif a.startswith("-") and a != "-":
            print("option %s not recognized" % a)
            print("use -h/--help for command line help")
            return
        else:
            stmt = a
        i += 1

    setup_src = "\n".join(setups) if setups else "pass"
    timer_fn = default_timer
    if _wrap_timer is not None:
        timer_fn = _wrap_timer(timer_fn)
    t = Timer(stmt=stmt or "pass", setup=setup_src, timer=timer_fn)

    if opts_number is None:
        num_loops, _t = t.autorange()
    else:
        num_loops = opts_number
    results = t.repeat(opts_repeat, num_loops)
    best = min(results)

    usecs = best * 1e6
    if unit is not None:
        scale = {"nsec": 1e-9, "usec": 1e-6, "msec": 1e-3, "sec": 1.0}[unit]
        printed = "%g %s" % (round(best / scale, 3), unit)
    else:
        printed = _format_time(usecs)

    loops_word = "loop" if num_loops == 1 else "loops"
    print("%d %s, best of %d: %s per loop"
          % (num_loops, loops_word, opts_repeat, printed))


if __name__ == "__main__":
    main()
