"""Minimal asyncio stub for Django 6.x"""

import sys

# Core event loop
class AbstractEventLoop:
    def run_until_complete(self, future):
        return future

    def run_forever(self):
        pass

    def stop(self):
        pass

    def close(self):
        pass

    def create_task(self, coro):
        return coro

    def call_soon(self, callback, *args):
        pass

    def call_later(self, delay, callback, *args):
        pass

class BaseEventLoop(AbstractEventLoop):
    pass

# Futures and Tasks
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

def sleep(delay, result=None):
    return Future()

def gather(*coros_or_futures, return_exceptions=False):
    results = []
    for c in coros_or_futures:
        try:
            results.append(c)
        except Exception as e:
            if return_exceptions:
                results.append(e)
            else:
                raise
    f = Future()
    f.set_result(results)
    return f

# Constants
ALL_COMPLETED = 'ALL_COMPLETED'
FIRST_COMPLETED = 'FIRST_COMPLETED'
FIRST_EXCEPTION = 'FIRST_EXCEPTION'

def wait(futures, *, return_when=ALL_COMPLETED):
    done = set()
    pending = set()
    for f in futures:
        if hasattr(f, 'done') and f.done():
            done.add(f)
        else:
            pending.add(f)
    return (done, pending)

# Event loop policy
class DefaultEventLoopPolicy:
    def get_event_loop(self):
        return BaseEventLoop()

    def new_event_loop(self):
        return BaseEventLoop()

    def set_event_loop(self, loop):
        pass

def get_event_loop():
    return BaseEventLoop()

def set_event_loop(loop):
    pass

def new_event_loop():
    return BaseEventLoop()

# Coroutines support
class coroutine:
    def __init__(self, func):
        self._func = func

    def __call__(self, *args, **kwargs):
        return self._func(*args, **kwargs)

def iscoroutine(obj):
    return isinstance(obj, coroutine) or hasattr(obj, '__await__')

def iscoroutinefunction(func):
    return hasattr(func, '__code__') or isinstance(func, coroutine)

# coroutines module
import asyncio.coroutines

# Queue
class Queue:
    def __init__(self, maxsize=0):
        self._queue = []
        self._maxsize = maxsize

    def put(self, item):
        self._queue.append(item)

    def get(self):
        if self._queue:
            return self._queue.pop(0)
        raise Exception('Queue empty')

# run function (Python 3.7+)
def run(main, *, debug=None):
    loop = get_event_loop()
    try:
        return loop.run_until_complete(main)
    finally:
        loop.close()
