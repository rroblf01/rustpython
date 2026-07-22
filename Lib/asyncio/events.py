# asyncio.events stub for RustPython
# Independent sub-module with event loop stubs.

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

class DefaultEventLoopPolicy:
    def get_event_loop(self):
        return BaseEventLoop()
    def new_event_loop(self):
        return BaseEventLoop()
    def set_event_loop(self, loop):
        pass

def get_event_loop():
    return BaseEventLoop()

def get_running_loop():
    # Real semantics: return the loop currently driving execution, or raise
    # RuntimeError if none is running. This stub has no real
    # coroutine/task-scheduling state to know "am I currently inside a
    # running loop", but the synchronous case (no loop running at all,
    # by far the most common — e.g. plain `django.setup()`) is always
    # correct to report this way. Missing this entirely (AttributeError
    # instead of RuntimeError) broke the extremely common defensive idiom
    # `try: loop = asyncio.get_running_loop() except RuntimeError: ...`,
    # since callers catching RuntimeError never expected an AttributeError.
    raise RuntimeError("no running event loop")

def set_event_loop(loop):
    pass

def new_event_loop():
    return BaseEventLoop()
