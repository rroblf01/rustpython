"""Selector module stub."""
import select


class BaseSelector:
    def __init__(self):
        pass
    def register(self, fileobj, events, data=None):
        pass
    def unregister(self, fileobj):
        pass
    def modify(self, fileobj, events, data=None):
        pass
    def select(self, timeout=None):
        return []
    def close(self):
        pass
    def get_map(self):
        return {}
    def __enter__(self):
        return self
    def __exit__(self, *args):
        self.close()


class SelectSelector(BaseSelector):
    pass


class PollSelector(BaseSelector):
    pass


class EpollSelector(BaseSelector):
    pass


class DevpollSelector(BaseSelector):
    pass


class KqueueSelector(BaseSelector):
    pass


DefaultSelector = SelectSelector


SELECT = 1
WRITE = 2
ERROR = 4
