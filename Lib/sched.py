"""Scheduler module."""

import time


class scheduler:
    def __init__(self, timefunc=time.monotonic, delayfunc=time.sleep):
        self._timefunc = timefunc
        self._delayfunc = delayfunc
        self._queue = []

    def enterabs(self, time, priority, action, arguments=(), kwargs={}):
        return Event(time, priority, action, arguments, kwargs, self)

    def enter(self, delay, priority, action, arguments=(), kwargs={}):
        return self.enterabs(self._timefunc() + delay, priority, action, arguments, kwargs)

    def cancel(self, event):
        self._queue.remove(event)

    def empty(self):
        return len(self._queue) == 0

    def run(self, blocking=True):
        while self._queue:
            self._queue.sort()
            event = self._queue.pop(0)
            if event.time > self._timefunc():
                self._delayfunc(event.time - self._timefunc())
            try:
                event.action(*event.arguments, **event.kwargs)
            except Exception:
                pass

    def queue(self):
        return self._queue[:]


class Event:
    def __init__(self, time, priority, action, arguments, kwargs, scheduler):
        self.time = time
        self.priority = priority
        self.action = action
        self.arguments = arguments
        self.kwargs = kwargs
        self.scheduler = scheduler

    def __lt__(self, other):
        return (self.time, self.priority) < (other.time, other.priority)

    def __eq__(self, other):
        return (self.time, self.priority) == (other.time, other.priority)
