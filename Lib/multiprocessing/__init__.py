"""Multiprocessing module stub."""

import threading
import sys


def cpu_count():
    import os
    try:
        return os.cpu_count() or 1
    except:
        return 1


class Process:
    def __init__(self, group=None, target=None, name=None, args=(), kwargs={}, *, daemon=None):
        self._target = target
        self._args = args
        self._kwargs = kwargs
        self._thread = None
        self._daemon = daemon or False

    def start(self):
        self._thread = threading.Thread(target=self._target, args=self._args, kwargs=self._kwargs)
        self._thread.daemon = self._daemon
        self._thread.start()

    def join(self, timeout=None):
        if self._thread:
            self._thread.join(timeout)

    def is_alive(self):
        return self._thread and self._thread.is_alive()

    def terminate(self):
        pass

    @property
    def pid(self):
        return 0

    @property
    def daemon(self):
        return self._daemon

    @daemon.setter
    def daemon(self, value):
        self._daemon = value
