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


def _candidate_filename(suffix="", prefix="tmp", dir=None):
    import uuid
    if dir is None:
        dir = gettempdir()
    name = dir + "/" + prefix + str(uuid.uuid4())[:8] + suffix
    return name


def mkstemp(suffix="", prefix="tmp", dir=None, text=False):
    if dir is None:
        dir = gettempdir()
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
    if dir is None:
        dir = gettempdir()
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
    if dir is None:
        dir = gettempdir()
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
                 delete=True, errors=None):
        self._delete = delete
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
            if self._delete:
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
    def __init__(self, suffix="", prefix="tmp", dir=None):
        self.name = mkdtemp(suffix, prefix, dir)

    def cleanup(self):
        import shutil
        try:
            shutil.rmtree(self.name)
        except Exception:
            pass

    def __enter__(self):
        return self.name

    def __exit__(self, exc, value, tb):
        self.cleanup()


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
