"""Temporary file utilities.

Minimal implementation sufficient for Django and common stdlib use.
"""

import os as _os
import io as _io

__all__ = [
    "NamedTemporaryFile", "TemporaryFile", "SpooledTemporaryFile",
    "mkstemp", "mkdtemp", "mktemp",
    "gettempdir", "gettempprefix",
    "tempdir", "template",
]

# ── Global state ─────────────────────────────────────────────────────────────

_os.name = "posix"  # Ensure posix mode

tempdir = None
template = "tmp"


def gettempdir():
    """Return the name of the directory used for temporary files."""
    global tempdir
    if tempdir is not None:
        return tempdir
    for var in ["TMPDIR", "TEMP", "TMP"]:
        val = _os.getenv(var)
        if val and _os.path.isdir(val):
            tempdir = val
            return tempdir
    for d in ["/tmp", "/var/tmp", "/usr/tmp"]:
        if _os.path.isdir(d):
            tempdir = d
            return d
    tempdir = "/tmp"
    return tempdir


def gettempprefix():
    """Return the filename prefix used for temporary files."""
    return template


# ── Random name generation ───────────────────────────────────────────────────


def _candidate_filename(suffix="", prefix="tmp", dir=None):
    """Generate a unique temporary filename."""
    import uuid
    if dir is None:
        dir = gettempdir()
    name = dir + "/" + prefix + str(uuid.uuid4())[:8] + suffix
    return name


# ── Low-level functions ──────────────────────────────────────────────────────


def mkstemp(suffix="", prefix="tmp", dir=None, text=False):
    """Create a temporary file and return (fd, name).

    The file is opened with O_CREAT | O_EXCL | O_RDWR.
    """
    if dir is None:
        dir = gettempdir()
    _os.makedirs(dir, exist_ok=True)

    name = _candidate_filename(suffix, prefix, dir)
    # Ensure we don't have race conditions
    try:
        fd = _os.open(name, _os.O_CREAT | _os.O_EXCL | _os.O_RDWR, 0o600)
        return (fd, name)
    except FileExistsError:
        # Extremely rare, try once more
        name = _candidate_filename(suffix, prefix, dir)
        fd = _os.open(name, _os.O_CREAT | _os.O_EXCL | _os.O_RDWR, 0o600)
        return (fd, name)


def mkdtemp(suffix="", prefix="tmp", dir=None):
    """Create a temporary directory and return its name."""
    if dir is None:
        dir = gettempdir()
    _os.makedirs(dir, exist_ok=True)

    name = _candidate_filename(suffix, prefix, dir)
    try:
        _os.mkdir(name, 0o700)
        return name
    except FileExistsError:
        name = _candidate_filename(suffix, prefix, dir)
        _os.mkdir(name, 0o700)
        return name


def mktemp(suffix="", prefix="tmp", dir=None):
    """Return a unique temporary filename without creating a file.

    Deprecated: use mkstemp instead.
    """
    if dir is None:
        dir = gettempdir()
    return _candidate_filename(suffix, prefix, dir)


# ── High-level classes ───────────────────────────────────────────────────────


class TemporaryFile:
    """Wrapper around a temporary file.

    Creates a real temp file (via mkstemp) and provides file-like access.
    """

    def __init__(self, mode="w+b", buffering=-1, encoding=None,
                 newline=None, suffix="", prefix="tmp", dir=None, errors=None):
        self._mode = mode
        fd, self.name = mkstemp(suffix, prefix, dir)
        self._file = _os.fdopen(fd, mode, buffering, encoding=encoding,
                                newline=newline, errors=errors)
        self._close_called = False

    def __getattr__(self, attr):
        return getattr(self._file, attr)

    def close(self):
        if not self._close_called:
            self._close_called = True
            try:
                self._file.close()
            except Exception:
                pass
            try:
                _os.unlink(self.name)
            except Exception:
                pass

    def __enter__(self):
        return self

    def __exit__(self, exc, value, tb):
        self.close()

    def __del__(self):
        self.close()


class NamedTemporaryFile:
    """A temporary file that will be removed when closed.

    This class is deliberately NOT a file object, but provides file-like
    access via delegation.
    """

    def __init__(self, mode="w+b", buffering=-1, encoding=None,
                 newline=None, suffix="", prefix="tmp", dir=None,
                 delete=True, errors=None):
        self._delete = delete
        self._close_called = False

        fd, self.name = mkstemp(suffix, prefix, dir)
        self._file = _os.fdopen(fd, mode, buffering, encoding=encoding,
                                newline=newline, errors=errors)

    def __getattr__(self, attr):
        """Delegate attribute access to the underlying file."""
        if attr in ("name", "_file", "_close_called", "_delete", "close",
                    "__enter__", "__exit__", "__del__"):
            raise AttributeError(attr)
        return getattr(self._file, attr)

    def close(self):
        if not self._close_called:
            self._close_called = True
            try:
                self._file.close()
            except Exception:
                pass
            if self._delete:
                try:
                    _os.unlink(self.name)
                except Exception:
                    pass

    def __enter__(self):
        return self

    def __exit__(self, exc, value, tb):
        self.close()

    def __del__(self):
        self.close()


class SpooledTemporaryFile(_io.BytesIO):
    """A temporary file that buffers in memory until it exceeds max_size."""

    def __init__(self, max_size=0, mode="w+b", buffering=-1, encoding=None,
                 newline=None, suffix="", prefix="tmp", dir=None, errors=None):
        self._max_size = max_size
        self._rolled = False
        self._file = _io.BytesIO()
        # ... minimal stub

    def write(self, data):
        if self._rolled:
            return self._file.write(data)
        return self._file.write(data)


# ── Cleanup / Compatibility ──────────────────────────────────────────────────


def _remove_all(*args, **kwargs):
    """Placeholder: cleanup of stale temp files."""
    pass
