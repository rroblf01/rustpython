"""Debugger framework."""

import fnmatch


class Bdb:
    """Generic Python debugger base class."""

    def __init__(self, skip=None):
        self.skip = set(skip) if skip else set()

    def canonic(self, filename):
        if not filename:
            return ''
        return filename

    def reset(self):
        self._wait_for_break = False
        self.quitting = False

    def trace_dispatch(self, frame, event, arg):
        if self.quitting:
            return
        if event == 'line':
            return self.dispatch_line(frame)
        if event == 'call':
            return self.dispatch_call(frame, arg)
        if event == 'return':
            return self.dispatch_return(frame, arg)
        if event == 'exception':
            return self.dispatch_exception(frame, arg)
        return self.trace_dispatch

    def dispatch_line(self, frame):
        if self.stop_here(frame):
            self.user_line(frame)
            if self.quitting:
                raise SystemExit
        return self.trace_dispatch

    def dispatch_call(self, frame, arg):
        if self.quitting:
            return
        return self.trace_dispatch

    def dispatch_return(self, frame, arg):
        if self.stop_here(frame):
            self.user_return(frame, arg)
            if self.quitting:
                raise SystemExit
        return self.trace_dispatch

    def dispatch_exception(self, frame, arg):
        if self.stop_here(frame):
            self.user_exception(frame, arg)
            if self.quitting:
                raise SystemExit
        return self.trace_dispatch

    def stop_here(self, frame):
        if self.quitting:
            return True
        return False

    def user_line(self, frame):
        pass

    def user_call(self, frame, argument_list):
        pass

    def user_return(self, frame, return_value):
        pass

    def user_exception(self, frame, exc_info):
        pass

    def user_exception(self, frame, exc_info):
        pass

    def set_quit(self):
        self.quitting = True
