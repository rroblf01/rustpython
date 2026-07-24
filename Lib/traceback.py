import sys
import itertools


def _exc_name(exc_type):
    return exc_type.__name__


def _format_exception_only_lines(exc_type, value):
    name = _exc_name(exc_type)
    lines = []
    if value is None:
        lines.append(f"{name}\n")
    else:
        lines.append(f"{name}: {value}\n")
    return lines


def format_exception_only(exc_type, value=None):
    parts = _format_exception_only_lines(exc_type, value)
    return "".join(parts)


class _ExceptionPrintContext:
    def __init__(self, exc_type, exc_value, exc_traceback, *, limit=None, chain=True):
        self.exc_type = exc_type
        self.exc_value = exc_value
        self.exc_traceback = exc_traceback
        self.limit = limit
        self.chain = chain

    def format(self, *, chain=True, colorize=False):
        return "".join(self.format_exception_only())

    def format_exception_only(self):
        return format_exception_only(self.exc_type, self.exc_value)


def format_exc(limit=None, chain=True):
    exc_type, exc_value, exc_tb = sys.exc_info()
    if exc_type is None:
        return "NoneType: None\n"
    return "".join(format_exception(exc_type, exc_value, exc_tb, limit=limit, chain=chain))


def print_exc(limit=None, file=None, chain=True):
    if file is None:
        file = sys.stderr
    s = format_exc(limit=limit, chain=chain)
    print(s, file=file, end="")


def format_exception(exc, /, value=None, tb=None, limit=None, chain=True):
    if value is None:
        value = exc
    if isinstance(exc, type):
        exc_type = exc
    else:
        exc_type = type(exc)
    ctx = _ExceptionPrintContext(exc_type, value, tb, limit=limit, chain=chain)
    return [ctx.format()]


def print_exception(exc, /, value=None, tb=None, limit=None, file=None, chain=True):
    if file is None:
        file = sys.stderr
    for line in format_exception(exc, value, tb, limit=limit, chain=chain):
        print(line, file=file, end="")


def format_tb(tb, limit=None):
    return []


def print_tb(tb, limit=None, file=None):
    pass


def clear_frames(tb):
    """Clear the local variables of all the frames in a traceback."""
    while tb is not None:
        try:
            tb.tb_frame.clear()
        except AttributeError:
            pass
        tb = tb.tb_next


class TracebackException:
    """An exception with a traceback, for formatting."""
    def __init__(self, exc_type, exc_value, exc_traceback, *, limit=None, capture_locals=False, compact=False):
        self.exc_type = exc_type
        self.exc_value = exc_value
        self.exc_traceback = exc_traceback
    def format(self, *, chain=True, colorize=False):
        return ["".join(self.format_exception_only())]
    def format_exception_only(self):
        return _format_exception_only_lines(self.exc_type, self.exc_value)
    def format_summary(self):
        return "".join(self.format_exception_only())


__all__ = [
    "TracebackException",
    "clear_frames", "format_exc", "format_exception", "format_exception_only",
    "format_tb", "print_exc", "print_exception", "print_tb",
]
