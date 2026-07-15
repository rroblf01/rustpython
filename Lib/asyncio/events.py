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

def set_event_loop(loop):
    pass

def new_event_loop():
    return BaseEventLoop()
