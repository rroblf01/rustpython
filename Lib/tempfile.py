"""Temporary file utilities."""

import os as _os
import io as _io
import atexit as _atexit

__all__ = [
    "NamedTemporaryFile", "TemporaryFile", "SpooledTemporaryFile",
    "TemporaryDirectory",
    "mkstemp", "mkdtemp", "mktemp",
    "gettempdir", "gettempprefix",
    "tempdir", "template",
]

_os.name = "posix"

tempdir = None
template = "tmp"

# Track created temp files/dirs for atexit cleanup
_temp_files = []
_temp_dirs = []


def gettempdir():
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
    return template


_text_openflags = _os.O_RDWR | _os.O_CREAT | _os.O_EXCL
if hasattr(_os, "O_NOFOLLOW"):
    _text_openflags |= _os.O_NOFOLLOW
_bin_openflags = _text_openflags


def _sanitize_params(suffix, prefix, dir):
    # Real CPython's `mkstemp`/`mkdtemp`/`mktemp`/`NamedTemporaryFile` all
    # accept `suffix=None`/`prefix=None` (not just an omitted argument) as
    # meaning "use the default" -- a very common calling idiom in real code
    # (e.g. `Lib/test/support/__init__.py`-style helpers that forward a
    # `prefix=None` parameter straight through). This simplified
    # reimplementation used mutable string defaults instead, so an
    # explicit `None` silently overrode them and blew up downstream in
    # `_candidate_filename`'s string concatenation with a
    # "'str' + 'NoneType'" TypeError instead of CPython's real behavior.
    if suffix is None:
        suffix = ""
    if prefix is None:
        prefix = gettempprefix()
    if dir is None:
        dir = gettempdir()
    return suffix, prefix, dir


def _candidate_filename(suffix="", prefix="tmp", dir=None):
    import uuid
    suffix, prefix, dir = _sanitize_params(suffix, prefix, dir)
    name = dir + "/" + prefix + str(uuid.uuid4())[:8] + suffix
    return name


def mkstemp(suffix="", prefix="tmp", dir=None, text=False):
    suffix, prefix, dir = _sanitize_params(suffix, prefix, dir)
    _os.makedirs(dir, exist_ok=True)
    name = _candidate_filename(suffix, prefix, dir)
    for _ in range(100):
        try:
            fd = _os.open(name, _os.O_CREAT | _os.O_EXCL | _os.O_RDWR, 0o600)
            _temp_files.append(name)
            return (fd, name)
        except OSError:
            name = _candidate_filename(suffix, prefix, dir)
    raise OSError("mkstemp: could not create unique temporary file")


def mkdtemp(suffix="", prefix="tmp", dir=None):
    suffix, prefix, dir = _sanitize_params(suffix, prefix, dir)
    _os.makedirs(dir, exist_ok=True)
    name = _candidate_filename(suffix, prefix, dir)
    for _ in range(100):
        try:
            _os.mkdir(name, 0o700)
            _temp_dirs.append(name)
            return name
        except OSError:
            name = _candidate_filename(suffix, prefix, dir)
    raise OSError("mkdtemp: could not create unique temporary directory")


def mktemp(suffix="", prefix="tmp", dir=None):
    suffix, prefix, dir = _sanitize_params(suffix, prefix, dir)
    return _candidate_filename(suffix, prefix, dir)


class TemporaryFile:
    def __init__(self, mode="w+b", buffering=-1, encoding=None,
                 newline=None, suffix="", prefix="tmp", dir=None, errors=None):
        self._mode = mode
        fd, self.name = mkstemp(suffix, prefix, dir)
        self._file = _os.fdopen(fd, mode, buffering=-1, encoding=encoding,
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


class NamedTemporaryFile:
    def __init__(self, mode="w+b", buffering=-1, encoding=None,
                 newline=None, suffix="", prefix="tmp", dir=None,
                 delete=True, delete_on_close=True, errors=None):
        self._delete = delete
        self.delete_on_close = delete_on_close
        self._close_called = False
        fd, self.name = mkstemp(suffix, prefix, dir)
        self._file = _os.fdopen(fd, mode, buffering=-1, encoding=encoding,
                                newline=newline, errors=errors)

    def __getattr__(self, attr):
        if attr in ("name", "_file", "_close_called", "_delete", "close",
                    "__enter__", "__exit__"):
            raise AttributeError(attr)
        return getattr(self._file, attr)

    def close(self):
        if not self._close_called:
            self._close_called = True
            try:
                self._file.close()
            except Exception:
                pass
            if self._delete and self.delete_on_close:
                try:
                    _os.unlink(self.name)
                except Exception:
                    pass

    def __enter__(self):
        return self

    def __exit__(self, exc, value, tb):
        self.close()


class SpooledTemporaryFile(_io.BytesIO):
    def __init__(self, max_size=0, mode="w+b", buffering=-1, encoding=None,
                 newline=None, suffix="", prefix="tmp", dir=None, errors=None):
        self._max_size = max_size
        self._rolled = False
        self._file = _io.BytesIO()

    def write(self, data):
        if self._rolled:
            return self._file.write(data)
        return self._file.write(data)


class TemporaryDirectory:
    # Real CPython's TemporaryDirectory accepts `ignore_cleanup_errors` (skip
    # cleanup failures instead of raising) and `delete` (opt out of removal
    # entirely) — this reimplementation previously only took
    # suffix/prefix/dir, so any real-world call like
    # `TemporaryDirectory(ignore_cleanup_errors=True)` raised
    # `TypeError: unexpected keyword argument`.
    def __init__(self, suffix="", prefix="tmp", dir=None,
                 ignore_cleanup_errors=False, delete=True):
        self.name = mkdtemp(suffix, prefix, dir)
        self._ignore_cleanup_errors = ignore_cleanup_errors
        self._delete = delete
        self._finalized = False

    def cleanup(self):
        # Idempotent: a caller doing `with TemporaryDirectory() as d: ...`
        # and *also* calling `.cleanup()` manually (or a GC-driven __del__
        # racing the `with`-exit) must not raise merely because the
        # directory is already gone — only report genuine cleanup failures.
        if not self._delete or self._finalized:
            return
        self._finalized = True
        import shutil
        if self._ignore_cleanup_errors:
            shutil.rmtree(self.name, ignore_errors=True)
        else:
            try:
                shutil.rmtree(self.name)
            except FileNotFoundError:
                pass

    def __enter__(self):
        return self.name

    def __exit__(self, exc, value, tb):
        self.cleanup()

    def __repr__(self):
        return f"<{self.__class__.__name__} {self.name!r}>"


class _TemporaryFileCloser:
    """A separate object allowing proper closing of a temporary file's underlying file."""

    cleanup_called = False
    close_called = False

    def __init__(self, file, name, delete=True, delete_on_close=True,
                 warn_message="Implicitly cleaning up unknown file"):
        self.file = file
        self.name = name
        self.delete = delete
        self.delete_on_close = delete_on_close
        self.warn_message = warn_message
        self.cleanup_called = False
        self.close_called = False

    def cleanup(self, windows=(_os.name == 'nt'), unlink=_os.unlink):
        if not self.cleanup_called:
            self.cleanup_called = True
            try:
                if not self.close_called:
                    self.close_called = True
                    try:
                        self.file.close()
                    except Exception:
                        pass
            finally:
                if self.delete and not (windows and self.delete_on_close):
                    try:
                        unlink(self.name)
                    except FileNotFoundError:
                        pass
                    except Exception:
                        pass

    def close(self):
        if not self.close_called:
            self.close_called = True
            try:
                try:
                    self.file.close()
                except Exception:
                    pass
            finally:
                if self.delete and self.delete_on_close:
                    self.cleanup()

    def __del__(self):
        close_called = self.close_called
        self.cleanup()
        if not close_called:
            try:
                import warnings as _warnings
                _warnings.warn(self.warn_message, ResourceWarning)
            except Exception:
                pass


class _TemporaryFileWrapper:
    """Temporary file wrapper — file-like object that deletes file on close."""

    def __init__(self, file, name, delete=True, delete_on_close=True):
        self.file = file
        self.name = name
        self._closer = _TemporaryFileCloser(
            file, name, delete, delete_on_close,
            warn_message=f"Implicitly cleaning up {self!r}",
        )

    def __repr__(self):
        file = self.__dict__['file']
        return f"<{type(self).__name__} {file=}>"

    def __getattr__(self, name):
        file = self.__dict__['file']
        a = getattr(file, name)
        if hasattr(a, '__call__'):
            func = a
            def func_wrapper(*args, **kwargs):
                return func(*args, **kwargs)
            try:
                import functools as _functools
                func_wrapper = _functools.wraps(func)(func_wrapper)
            except Exception:
                pass
            func_wrapper._closer = self._closer
            a = func_wrapper
        if not isinstance(a, int):
            try:
                setattr(self, name, a)
            except Exception:
                pass
        return a

    def __enter__(self):
        self.file.__enter__()
        return self

    def __exit__(self, exc, value, tb):
        result = self.file.__exit__(exc, value, tb)
        self._closer.cleanup()
        return result

    def close(self):
        self._closer.close()

    def __iter__(self):
        for line in self.file:
            yield line


def _remove_all(*args, **kwargs):
    for path in _temp_files:
        try:
            _os.unlink(path)
        except Exception:
            pass
    _temp_files.clear()
    import shutil
    for path in _temp_dirs:
        try:
            shutil.rmtree(path)
        except Exception:
            pass
    _temp_dirs.clear()


_atexit.register(_remove_all)
