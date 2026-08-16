import sys
import itertools


def _exc_name(exc_type):
    try:
        return exc_type.__name__
    except AttributeError:
        return str(exc_type)


def _format_exception_only_lines(exc_type, value):
    name = _exc_name(exc_type)
    lines = []
    if value is None:
        lines.append(f"{name}\n")
    else:
        try:
            msg = str(value)
        except Exception:
            msg = "<exception str() failed>"
        if msg:
            lines.append(f"{name}: {msg}\n")
        else:
            lines.append(f"{name}\n")
    return lines


def format_exception_only(exc_type, value=None):
    parts = _format_exception_only_lines(exc_type, value)
    return "".join(parts)


class FrameSummary:
    """One frame of a traceback: filename, lineno, name, and source line."""

    def __init__(self, filename, lineno, name, line=None, end_lineno=None, end_colno=None, colno=None):
        self.filename = filename
        self.lineno = lineno
        self.name = name
        self.line = line
        self.end_lineno = end_lineno
        self.end_colno = end_colno
        self.colno = colno

    def __eq__(self, other):
        return (self.filename, self.lineno, self.name) == (other.filename, other.lineno, other.name)

    def __hash__(self):
        return hash((self.filename, self.lineno, self.name))


def _get_line(filename, lineno):
    """Read the source line at (filename, lineno) if the file is readable."""
    try:
        with open(filename, "r", errors="replace") as f:
            lines = f.readlines()
        if 1 <= lineno <= len(lines):
            return lines[lineno - 1].rstrip("\n")
    except OSError:
        pass
    except Exception:
        pass
    return None


def extract_tb(tb, limit=None):
    """Return a list of FrameSummary for the (real) traceback chain."""
    result = []
    count = 0
    while tb is not None and (limit is None or count < limit):
        frame = getattr(tb, "tb_frame", None)
        lineno = getattr(tb, "tb_lineno", None)
        if frame is not None:
            code = getattr(frame, "f_code", None)
            if code is not None:
                filename = getattr(code, "co_filename", None) or "<unknown>"
                name = getattr(code, "co_name", None) or "?"
            else:
                filename = "<unknown>"
                name = "?"
        else:
            filename = "<unknown>"
            name = "?"
        result.append(FrameSummary(filename, lineno or 0, name, _get_line(filename, lineno)))
        count += 1
        tb = getattr(tb, "tb_next", None)
    return result


def format_list(extracted_list):
    """Format a list of FrameSummary/tuples into lines."""
    result = []
    for item in extracted_list:
        if isinstance(item, FrameSummary):
            filename, lineno, name, line = item.filename, item.lineno, item.name, item.line
        else:
            filename, lineno, name, line = item
        if lineno:
            result.append(f'  File "{filename}", line {lineno}, in {name}\n')
        else:
            result.append(f'  File "{filename}", in {name}\n')
        if line:
            result.append(f"    {line}\n")
    return result


def format_tb(tb, limit=None):
    return format_list(extract_tb(tb, limit))


def print_tb(tb, limit=None, file=None):
    if file is None:
        file = sys.stderr
    for line in format_tb(tb, limit):
        print(line, file=file, end="")


def format_exception(exc, /, value=None, tb=None, limit=None, chain=True):
    """Full 'Traceback (most recent call last): ... Type: message' report."""
    if value is None:
        value = exc
    if isinstance(exc, type):
        exc_type = exc
    else:
        exc_type = type(exc)
    out = []
    # Implicit __context__ chaining: walk the context chain like CPython.
    seen = set()
    e, t = value, tb
    while e is not None:
        if id(e) in seen:
            break
        seen.add(id(e))
        ctx = None
        try:
            ctx = e.__context__
            cause = e.__cause__
            suppressed = e.__suppress_context__
        except AttributeError:
            ctx = None
            cause = None
            suppressed = False
        if cause is not None and id(cause) not in seen:
            # explicit cause
            out.append("The above exception was the direct cause of the "
                       "following exception:\n\n")
            t = getattr(cause, "__traceback__", None)
            e = cause
            continue
        if ctx is not None and not suppressed and id(ctx) not in seen:
            out.append("During handling of the above exception, another "
                       "exception occurred:\n\n")
            t = getattr(ctx, "__traceback__", None)
            e = ctx
            continue
        break
    out.append("Traceback (most recent call last):\n")
    out.extend(format_tb(t, limit))
    out.extend(_format_exception_only_lines(exc_type, value))
    return out


def print_exception(exc, /, value=None, tb=None, limit=None, file=None, chain=True):
    if file is None:
        file = sys.stderr
    for line in format_exception(exc, value, tb, limit=limit, chain=chain):
        print(line, file=file, end="")


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

    def __init__(self, exc_type, exc_value, exc_traceback, *, limit=None,
                 capture_locals=False, compact=False):
        self.exc_type = exc_type
        self.exc_value = exc_value
        self.exc_traceback = exc_traceback
        self.limit = limit
        self.stack = extract_tb(exc_traceback, limit)

    def format(self, *, chain=True, colorize=False):
        return format_exception(self.exc_type, self.exc_value, self.exc_traceback,
                                limit=self.limit, chain=chain)

    def format_exception_only(self):
        return _format_exception_only_lines(self.exc_type, self.exc_value)

    def format_summary(self):
        return "".join(self.format_exception_only())


__all__ = [
    "FrameSummary", "TracebackException", "clear_frames", "extract_tb",
    "format_exc", "format_exception", "format_exception_only", "format_list",
    "format_tb", "print_exc", "print_exception", "print_tb",
]
