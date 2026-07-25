"""Logging module."""
import sys
import os
import time

CRITICAL = 50
ERROR = 40
WARNING = 30
INFO = 20
DEBUG = 10
NOTSET = 0


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
    def setFormatter(self, formatter):
        self.formatter = formatter
    def handle(self, record):
        if record.level >= self.level:
            self.emit(record)
    def emit(self, record):
        raise NotImplementedError


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


_root = None
manager = None


class Logger:
    def __init__(self, name, level=NOTSET):
        self.name = name
        self.level = level
        self.parent = None
        self.handlers = []
        self.disabled = False
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
