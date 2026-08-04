"""Minimal concurrent.futures.thread for RustPython.

Real CPython's thread.py implements ThreadPoolExecutor; this interpreter's
executor is native. Only `_WorkItem` is provided here — it's what real
modules import from this submodule (test_genericalias uses it as a
GenericAlias origin).
"""


class _WorkItem:
    def __init__(self, future, fn, args, kwargs):
        self.future = future
        self.fn = fn
        self.args = args
        self.kwargs = kwargs
