"""Logging module."""
import sys
import os
import time
import threading

CRITICAL = 50
FATAL = CRITICAL
ERROR = 40
WARNING = 30
WARN = WARNING
INFO = 20
DEBUG = 10
NOTSET = 0
BASIC_FORMAT = "%(message)s"


class LogRecord:
    def __init__(self, name, level, pathname, lineno, msg, args, exc_info, func=None, sinfo=None):
        self.name = name
        self.level = level
        self.pathname = pathname
        self.lineno = lineno
        self.msg = msg
        self.args = args
        self.exc_info = exc_info
        self.created = time.time()
        self.thread = 0
        self.process = 0

    def getMessage(self):
        msg = str(self.msg)
        if self.args:
            msg = msg % self.args
        return msg


class Handler:
    def __init__(self, level=NOTSET):
        self.level = level
        self.formatter = None
        self._closed = False
    def setFormatter(self, formatter):
        self.formatter = formatter
    def handle(self, record):
        if record.level >= self.level:
            self.emit(record)
    def emit(self, record):
        raise NotImplementedError
    def close(self):
        self._closed = True
    def flush(self):
        pass
    def createLock(self):
        self.lock = None


class Filter:
    """Record filter by logger name (CPython API subset)."""

    def __init__(self, name=""):
        self.name = name
        self.nlen = len(name)

    def filter(self, record):
        if self.nlen == 0:
            return True
        if self.name == record.name:
            return True
        if record.name.find(self.name, 0, self.nlen) != 0:
            return False
        return record.name[self.nlen] == "."

    def __repr__(self):
        return f"<logging.Filter object at 0x{id(self):x}>"


class NullHandler(Handler):
    """A do-nothing handler, for library code that wants a default handler
    to avoid the "No handlers could be found" warning without actually
    logging anywhere — matches real CPython's `logging.NullHandler`.
    """
    def handle(self, record):
        pass
    def emit(self, record):
        pass
    def createLock(self):
        self.lock = None


class StreamHandler(Handler):
    def __init__(self, stream=None):
        super().__init__()
        if stream is None:
            stream = sys.stderr
        self.stream = stream
    def emit(self, record):
        self.stream.write(self.format(record) + '\n')
    def format(self, record):
        if self.formatter:
            return self.formatter.format(record)
        return '{}:{}:{}'.format(record.level, record.name, record.getMessage())


class FileHandler(StreamHandler):
    def __init__(self, filename, mode='a', encoding=None, delay=False):
        self.filename = filename
        self.mode = mode
        self.encoding = encoding
        self.stream = None
        if not delay:
            self._open()
    def _open(self):
        self.stream = open(self.filename, self.mode, encoding=self.encoding)
    def emit(self, record):
        if self.stream is None:
            self._open()
        super().emit(record)


class Formatter:
    def __init__(self, fmt=None, datefmt=None):
        self._fmt = fmt
        self.datefmt = datefmt
    def format(self, record):
        return record.getMessage()


class BufferingFormatter:
    """Format multiple records (CPython API subset)."""

    def __init__(self, linefmt=None):
        if linefmt:
            self.linefmt = linefmt
        else:
            self.linefmt = Formatter()

    def formatHeader(self, records):
        return ""

    def formatFooter(self, records):
        return ""

    def format(self, records):
        rv = ""
        if len(records) > 0:
            rv += self.formatHeader(records)
            for record in records:
                rv += self.linefmt.format(record)
            rv += self.formatFooter(records)
        return rv


_root = None
manager = None


class Logger:
    def __init__(self, name, level=NOTSET):
        self.name = name
        self.level = level
        self.parent = None
        self.handlers = []
        self.disabled = False
        # CPython sets .manager (global Manager singleton); tests read it.
        global manager
        if manager is None:
            manager = type('Manager', (), {'loggerDict': {}})()
        self.manager = manager
    def setLevel(self, level):
        self.level = level
    def addHandler(self, handler):
        self.handlers.append(handler)
    def removeHandler(self, handler):
        self.handlers.remove(handler)
    def isEnabledFor(self, level):
        return level >= self.level
    def _log(self, level, msg, args, exc_info=None, extra=None):
        if self.disabled:
            return
        record = LogRecord(self.name, level, '', 0, msg, args, exc_info)
        self.handle(record)
    def handle(self, record):
        if record.level >= self.level:
            for handler in self.handlers:
                handler.handle(record)
    def debug(self, msg, *args, **kwargs):
        if self.isEnabledFor(DEBUG):
            self._log(DEBUG, msg, args, **kwargs)
    def info(self, msg, *args, **kwargs):
        if self.isEnabledFor(INFO):
            self._log(INFO, msg, args, **kwargs)
    def warning(self, msg, *args, **kwargs):
        if self.isEnabledFor(WARNING):
            self._log(WARNING, msg, args, **kwargs)
    def error(self, msg, *args, **kwargs):
        if self.isEnabledFor(ERROR):
            self._log(ERROR, msg, args, **kwargs)
    def critical(self, msg, *args, **kwargs):
        if self.isEnabledFor(CRITICAL):
            self._log(CRITICAL, msg, args, **kwargs)
    def log(self, level, msg, *args, **kwargs):
        if self.isEnabledFor(level):
            self._log(level, msg, args, **kwargs)
    def exception(self, msg, *args, exc_info=True, **kwargs):
        self.error(msg, *args, exc_info=exc_info, **kwargs)
    def fatal(self, msg, *args, **kwargs):
        self.critical(msg, *args, **kwargs)


class LoggerAdapter:
    """Prefix/extra-data adapter wrapping a Logger (CPython API subset)."""

    def __init__(self, logger, extra=None):
        self.logger = logger
        self.extra = extra

    def process(self, msg, kwargs):
        return msg, kwargs

    def setLevel(self, level):
        self.logger.setLevel(level)

    def debug(self, msg, *args, **kwargs):
        self.log(DEBUG, msg, *args, **kwargs)

    def info(self, msg, *args, **kwargs):
        self.log(INFO, msg, *args, **kwargs)

    def warning(self, msg, *args, **kwargs):
        self.log(WARNING, msg, *args, **kwargs)

    def error(self, msg, *args, **kwargs):
        self.log(ERROR, msg, *args, **kwargs)

    def exception(self, msg, *args, exc_info=True, **kwargs):
        self.error(msg, *args, exc_info=exc_info, **kwargs)

    def critical(self, msg, *args, **kwargs):
        self.log(CRITICAL, msg, *args, **kwargs)

    def log(self, level, msg, *args, **kwargs):
        if self.logger.isEnabledFor(level):
            msg, kwargs = self.process(msg, kwargs)
            self.logger._log(level, msg, args, **kwargs)

    def isEnabledFor(self, level):
        return self.logger.isEnabledFor(level)

    def getEffectiveLevel(self):
        return self.logger.level

    @property
    def name(self):
        return self.logger.name


def getLogger(name=None):
    global _root, manager
    if _root is None:
        _root = Logger('root', WARNING)
        _root.addHandler(StreamHandler())
        manager = type('Manager', (), {'loggerDict': {}})()
    if name is None:
        return _root
    if name not in manager.loggerDict:
        logger = Logger(name)
        logger.parent = _root
        manager.loggerDict[name] = logger
    return manager.loggerDict[name]


def basicConfig(**kwargs):
    global _root
    if _root is None:
        _root = Logger('root', WARNING)
    _root.handlers.clear()
    if 'filename' in kwargs:
        handler = FileHandler(kwargs['filename'])
    else:
        handler = StreamHandler()
    if 'level' in kwargs:
        _root.setLevel(kwargs['level'])
    _root.addHandler(handler)


def shutdown():
    if _root:
        for handler in _root.handlers:
            if hasattr(handler.stream, 'close'):
                handler.stream.close()


def _ensure_configured():
    if len(root.handlers) == 0:
        basicConfig()


def debug(msg, *args, **kwargs):
    _ensure_configured()
    getLogger().debug(msg, *args, **kwargs)


def info(msg, *args, **kwargs):
    _ensure_configured()
    getLogger().info(msg, *args, **kwargs)


def warning(msg, *args, **kwargs):
    _ensure_configured()
    getLogger().warning(msg, *args, **kwargs)


def warn(msg, *args, **kwargs):
    _ensure_configured()
    warning(msg, *args, **kwargs)


def error(msg, *args, **kwargs):
    _ensure_configured()
    getLogger().error(msg, *args, **kwargs)


def critical(msg, *args, **kwargs):
    _ensure_configured()
    getLogger().critical(msg, *args, **kwargs)


def fatal(msg, *args, **kwargs):
    _ensure_configured()
    critical(msg, *args, **kwargs)


def exception(msg, *args, exc_info=True, **kwargs):
    _ensure_configured()
    getLogger().exception(msg, *args, exc_info=exc_info, **kwargs)


def log(level, msg, *args, **kwargs):
    _ensure_configured()
    getLogger().log(level, msg, *args, **kwargs)


def disable(level=CRITICAL):
    getLogger().manager.disable = level


_level_names = {
    CRITICAL: 'CRITICAL',
    ERROR: 'ERROR',
    WARNING: 'WARNING',
    INFO: 'INFO',
    DEBUG: 'DEBUG',
    NOTSET: 'NOTSET',
    'CRITICAL': CRITICAL,
    'ERROR': ERROR,
    'WARN': WARNING,
    'WARNING': WARNING,
    'INFO': INFO,
    'DEBUG': DEBUG,
    'NOTSET': NOTSET,
}


def getLevelName(level):
    return _level_names.get(level, f'Level {level}')


# Module-level names CPython exposes (test_logging reads them directly).
_handlerList = []
_lock = threading.RLock()
_handlers = {}

__all__ = [
    'BASIC_FORMAT', 'BufferingFormatter', 'CRITICAL', 'DEBUG', 'ERROR',
    'FATAL', 'FileHandler', 'Filter', 'Formatter', 'Handler', 'INFO',
    'LogRecord', 'Logger', 'LoggerAdapter', 'NOTSET', 'NullHandler',
    'StreamHandler', 'WARN', 'WARNING', 'basicConfig', 'getLogger',
    'makeLogRecord', 'shutdown',
]


def makeLogRecord(dict):
    rv = LogRecord(None, None, "", 0, "", (), None, None)
    rv.__dict__.update(dict)
    return rv


# Eager root logger so `logging.root` exists without calling getLogger.
if manager is None:
    manager = type('Manager', (), {'loggerDict': {}})()
if _root is None:
    _root = Logger('root', WARNING)
    _root.addHandler(StreamHandler())
root = _root
