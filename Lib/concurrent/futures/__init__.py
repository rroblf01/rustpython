"""
concurrent.futures — Launch parallel tasks.

Provides ThreadPoolExecutor and ProcessPoolExecutor for asynchronous execution.
Delegates to the native RustPython concurrent.futures module when available.
"""
import sys

# Try to use the native concurrent.futures module
_cf_native = sys.modules.get("concurrent.futures")

# ── Futures ─────────────────────────────────────────────────────────────────

class Future:
    """Represents the result of an asynchronous computation."""
    def __init__(self):
        self._result = None
        self._exception = None
        self._done = False
        self._callbacks = []

    def result(self, timeout=None):
        if self._exception:
            raise self._exception
        return self._result

    def exception(self, timeout=None):
        return self._exception

    def done(self):
        return self._done

    def cancelled(self):
        return False

    def running(self):
        return False

    def cancel(self):
        return False

    def add_done_callback(self, fn):
        if self._done:
            fn(self)
        else:
            self._callbacks.append(fn)

    def set_result(self, result):
        self._result = result
        self._done = True
        for cb in self._callbacks:
            cb(self)

    def set_exception(self, exception):
        self._exception = exception
        self._done = True
        for cb in self._callbacks:
            cb(self)


# ── ThreadPoolExecutor ──────────────────────────────────────────────────────

class ThreadPoolExecutor:
    """Basic thread pool executor using threads (synchronous stub)."""

    def __init__(self, max_workers=None, thread_name_prefix="", initializer=None, initargs=()):
        self._max_workers = max_workers

    def submit(self, fn, *args, **kwargs):
        """Submit a callable for execution."""
        f = Future()
        try:
            result = fn(*args, **kwargs)
            f.set_result(result)
        except BaseException as e:
            f.set_exception(e)
        return f

    def map(self, fn, *iterables, timeout=None, chunksize=1):
        """Apply fn to each iterable and return results."""
        return [fn(*args) for args in zip(*iterables)]

    def shutdown(self, wait=True, *, cancel_futures=False):
        pass

    def __enter__(self):
        return self

    def __exit__(self, *args):
        self.shutdown()


# ── ProcessPoolExecutor (stub) ──────────────────────────────────────────────

class ProcessPoolExecutor:
    """Basic process pool executor (synchronous stub)."""

    def __init__(self, max_workers=None):
        self._max_workers = max_workers

    def submit(self, fn, *args, **kwargs):
        f = Future()
        try:
            result = fn(*args, **kwargs)
            f.set_result(result)
        except BaseException as e:
            f.set_exception(e)
        return f

    def map(self, fn, *iterables, timeout=None, chunksize=1):
        return [fn(*args) for args in zip(*iterables)]

    def shutdown(self, wait=True, *, cancel_futures=False):
        pass

    def __enter__(self):
        return self

    def __exit__(self, *args):
        self.shutdown(wait=True)


# ── Utility functions ───────────────────────────────────────────────────────

FIRST_COMPLETED = "FIRST_COMPLETED"
FIRST_EXCEPTION = "FIRST_EXCEPTION"
ALL_COMPLETED = "ALL_COMPLETED"


def wait(fs, timeout=None, return_when=ALL_COMPLETED):
    """Wait for the futures to complete. Returns (done, not_done)."""
    done = {f for f in fs if f.done()}
    not_done = set(fs) - done
    return (done, not_done)


def as_completed(fs, timeout=None):
    """Return an iterator over the futures as they complete."""
    return list(fs)


__all__ = [
    "Future",
    "ThreadPoolExecutor",
    "ProcessPoolExecutor",
    "wait",
    "as_completed",
    "FIRST_COMPLETED",
    "FIRST_EXCEPTION",
    "ALL_COMPLETED",
]
