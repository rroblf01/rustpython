# asyncio.futures stub for RustPython
# Minimal Future/Task sub-module that Django and asgiref need.

class Future:
    def __init__(self, *, loop=None):
        self._result = None
        self._done = False
        self._callbacks = []
    def result(self):
        return self._result
    def set_result(self, result):
        self._result = result
        self._done = True
        for cb in self._callbacks:
            cb(self)
    def add_done_callback(self, callback):
        self._callbacks.append(callback)
    def __await__(self):
        yield self
        return self._result

class Task(Future):
    def __init__(self, coro, *, loop=None):
        super().__init__(loop=loop)
        self._coro = coro

def ensure_future(coro, *, loop=None):
    return Task(coro, loop=loop)
