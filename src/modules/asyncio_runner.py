import asyncio as _asyncio_mod


class _RunnerLoop:
    """Minimal placeholder "event loop" object for Runner.get_loop().

    This project's native `asyncio` module has no real event loop or
    scheduler at all -- `asyncio.run()` executes a coroutine's frame
    directly and synchronously (see src/modules/misc/asyncio.rs). Real code
    that merely stores/inspects the object `Runner.get_loop()` returns
    (rather than driving it itself) still needs *something* loop-shaped to
    hold onto, so this exists purely as that placeholder.
    """

    def is_closed(self):
        return self._closed

    def is_running(self):
        return False

    def close(self):
        self._closed = True

    def __init__(self):
        self._closed = False


class Runner:
    """A context manager that controls event loop life cycle.

    Mirrors real CPython's `asyncio.Runner` API shape (debug=/loop_factory=
    constructor kwargs, __enter__/__exit__, close(), get_loop(),
    run(coro, context=None)). Since this simplified asyncio has no real
    event loop/scheduler underneath it, `run()` delegates to the existing,
    already-working `asyncio.run()` machinery (a direct, synchronous
    execution of the coroutine's frame) rather than driving a real loop --
    this class exists so real code can reference/use `asyncio.Runner` at
    all (e.g. `unittest.IsolatedAsyncioTestCase`), not to add real
    concurrency this project's asyncio doesn't have anywhere else either.
    """

    def __init__(self, *, debug=None, loop_factory=None):
        self._debug = debug
        self._loop_factory = loop_factory
        self._loop = None
        self._closed = False

    def __enter__(self):
        if self._closed:
            raise RuntimeError("Runner is closed")
        return self

    def __exit__(self, exc_type, exc_val, exc_tb):
        self.close()

    def close(self):
        """Shutdown and close event loop."""
        if self._closed:
            return
        if self._loop is not None:
            self._loop.close()
        self._closed = True
        self._loop = None

    def get_loop(self):
        """Return embedded event loop."""
        if self._closed:
            raise RuntimeError("Runner is closed")
        if self._loop is None:
            if self._loop_factory is not None:
                self._loop = self._loop_factory()
            else:
                self._loop = _RunnerLoop()
        return self._loop

    def run(self, coro, *, context=None):
        """Run a coroutine inside this Runner's (nominal) event loop."""
        if self._closed:
            raise RuntimeError("Runner is closed")
        # No real running-loop tracking to consult here (see
        # get_running_loop()'s own doc comment in asyncio.rs) -- this
        # simplified asyncio can never actually be "inside" a running loop
        # at the point .run() is called, so the real implementation's
        # matching RuntimeError guard has nothing to check against.
        self.get_loop()
        return _asyncio_mod.run(coro)
