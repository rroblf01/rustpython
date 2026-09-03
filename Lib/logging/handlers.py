"""Logging handlers module."""
from logging import Handler, LogRecord
import os


class RotatingFileHandler(Handler):
    def __init__(self, filename, mode='a', maxBytes=0, backupCount=0, encoding=None, delay=False):
        super().__init__()
        self.filename = filename
        self.mode = mode
        self.maxBytes = maxBytes
        self.backupCount = backupCount
        self.encoding = encoding
        self.stream = open(filename, mode, encoding=encoding)

    def emit(self, record):
        msg = self.format(record)
        self.stream.write(msg + '\n')
        if self.maxBytes > 0:
            self.stream.flush()
            if self.stream.tell() > self.maxBytes:
                self.doRollover()

    def doRollover(self):
        self.stream.close()
        if self.backupCount > 0:
            for i in range(self.backupCount - 1, 0, -1):
                sfn = f'{self.filename}.{i}'
                dfn = f'{self.filename}.{i + 1}'
                if os.path.exists(sfn):
                    os.rename(sfn, dfn)
            dfn = f'{self.filename}.1'
            if os.path.exists(self.filename):
                os.rename(self.filename, dfn)
        self.stream = open(self.filename, self.mode, encoding=self.encoding)


class TimedRotatingFileHandler(Handler):
    def __init__(self, filename, when='h', interval=1, backupCount=0, encoding=None, delay=False, utc=False, atTime=None):
        super().__init__()
        self.filename = filename
        self.stream = open(filename, 'a', encoding=encoding)

    def emit(self, record):
        msg = self.format(record)
        self.stream.write(msg + '\n')


class SocketHandler(Handler):
    def __init__(self, host, port):
        super().__init__()
        self.host = host
        self.port = port

    def emit(self, record):
        pass


class HTTPHandler(Handler):
    def __init__(self, host, url, method='GET'):
        super().__init__()
        self.host = host
        self.url = url
        self.method = method

    def emit(self, record):
        pass


class BufferingHandler(Handler):
    """Buffer log records; flush when capacity reached (CPython API)."""

    def __init__(self, capacity):
        super().__init__()
        self.capacity = capacity
        self.buffer = []

    def shouldFlush(self, record):
        return len(self.buffer) >= self.capacity

    def emit(self, record):
        self.buffer.append(record)
        if self.shouldFlush(record):
            self.flush()

    def flush(self):
        self.buffer.clear()


class QueueListener:
    """Monitor a queue for records, dispatching to handlers in a thread."""

    _sentinel = object()

    def __init__(self, queue, *handlers, respect_handler_level=False):
        self.queue = queue
        self.handlers = handlers
        self._thread = None
        self.respect_handler_level = respect_handler_level

    def _monitor(self):
        q = self.queue
        while True:
            try:
                record = q.get()
            except Exception:
                break
            if record is self._sentinel:
                break
            self.handle(record)

    def handle(self, record):
        for handler in self.handlers:
            if not self.respect_handler_level:
                handler.handle(record)
            else:
                if record.level >= handler.level:
                    handler.handle(record)

    def start(self):
        import threading
        self._thread = threading.Thread(target=self._monitor)
        self._thread.daemon = True
        self._thread.start()

    def stop(self):
        self.enqueue_sentinel()
        if self._thread is not None:
            self._thread.join()
            self._thread = None

    def enqueue_sentinel(self):
        self.queue.put(self._sentinel)


class MemoryHandler(Handler):
    def __init__(self, capacity, flushLevel=0, target=None):
        super().__init__()
        self.capacity = capacity
        self.buffer = []
        self.target = target

    def emit(self, record):
        self.buffer.append(record)
        if len(self.buffer) >= self.capacity:
            self.flush()

    def flush(self):
        if self.target:
            for record in self.buffer:
                self.target.handle(record)
            self.buffer.clear()


class QueueHandler(Handler):
    def __init__(self, queue):
        super().__init__()
        self.queue = queue

    def emit(self, record):
        self.queue.put(record)
