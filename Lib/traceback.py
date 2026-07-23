"""Pragmatic traceback module.

Not a vendor of real CPython's traceback.py (that version walks real frame/
tb_next chains this interpreter doesn't fully expose to Python code) — this
covers the interface shape real code actually calls (`format_exc`,
`print_exc`, `TracebackException`, `format_exception`, `print_exception`)
using whatever information is actually available (exception type name,
str(value), and this interpreter's own file/line/function chain when given).
"""

import sys


def _exc_name(exc_type):
    return getattr(exc_type, '__name__', str(exc_type))


def _format_exception_only_lines(exc_type, value):
    name = _exc_name(exc_type)
    msg = str(value) if value is not None else ''
    if msg:
        yield "%s: %s\n" % (name, msg)
    else:
        yield "%s\n" % (name,)


def format_exception_only(exc_type, value=None):
    if value is None and not isinstance(exc_type, type):
        # Support the 3.10+ format_exception_only(exc) single-arg form.
        value = exc_type
        exc_type = type(value)
    return list(_format_exception_only_lines(exc_type, value))


class TracebackException:
    """Simplified stand-in for real CPython's TracebackException.

    Captures just enough (exception type name, message, __cause__/
    __context__) to let code that only calls `.format()`/iterates the
    result work without crashing — this interpreter doesn't expose a
    walkable real traceback object chain to Python code, so per-frame
    formatting isn't attempted.
    """

    def __init__(self, exc_type, exc_value, exc_traceback, *, limit=None,
                 lookup_lines=True, capture_locals=False, compact=False,
                 max_group_width=15, max_group_depth=10, save_exc_type=True,
                 _seen=None):
        self.exc_type = exc_type
        self._str = str(exc_value) if exc_value is not None else ''
        self.__cause__ = None
        self.__context__ = None
        self.__suppress_context__ = getattr(exc_value, '__suppress_context__', False)
        cause = getattr(exc_value, '__cause__', None)
        if cause is not None:
            self.__cause__ = TracebackException(type(cause), cause, getattr(cause, '__traceback__', None),
                                                 capture_locals=capture_locals, compact=compact)
        context = getattr(exc_value, '__context__', None)
        if context is not None and context is not cause:
            self.__context__ = TracebackException(type(context), context, getattr(context, '__traceback__', None),
                                                    capture_locals=capture_locals, compact=compact)
        self.stack = []

    def format(self, *, chain=True, colorize=False):
        if chain:
            if self.__cause__ is not None:
                yield from self.__cause__.format(chain=chain, colorize=colorize)
                yield "\nThe above exception was the direct cause of the following exception:\n\n"
            elif self.__context__ is not None and not self.__suppress_context__:
                yield from self.__context__.format(chain=chain, colorize=colorize)
                yield "\nDuring handling of the above exception, another exception occurred:\n\n"
        yield "Traceback (most recent call last):\n"
        yield from _format_exception_only_lines(self.exc_type, self._str)

    def format_exception_only(self):
        return _format_exception_only_lines(self.exc_type, self._str)


def format_exc(limit=None, chain=True):
    exc_type, value, tb = sys.exc_info()
    if exc_type is None:
        return "NoneType: None\n"
    return "".join(TracebackException(exc_type, value, tb, compact=True).format(chain=chain))


def print_exc(limit=None, file=None, chain=True):
    if file is None:
        file = sys.stderr
    file.write(format_exc(limit=limit, chain=chain))


def format_exception(exc, /, value=None, tb=None, limit=None, chain=True):
    # Support both the legacy 3-positional-arg form
    # (format_exception(etype, value, tb)) and the modern single-exception
    # form (format_exception(exc)).
    if value is None and tb is None and not isinstance(exc, type):
        exc_type, exc_value, exc_tb = type(exc), exc, getattr(exc, '__traceback__', None)
    else:
        exc_type, exc_value, exc_tb = exc, value, tb
    if exc_type is None:
        return ["NoneType: None\n"]
    return list(TracebackException(exc_type, exc_value, exc_tb, compact=True).format(chain=chain))


def print_exception(exc, /, value=None, tb=None, limit=None, file=None, chain=True):
    if file is None:
        file = sys.stderr
    for line in format_exception(exc, value, tb, limit=limit, chain=chain):
        file.write(line)


def format_tb(tb, limit=None):
    return []


def print_tb(tb, limit=None, file=None):
    pass
