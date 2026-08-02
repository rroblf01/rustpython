"""Trace malloc module stub."""


class Trace:
    pass


def get_object_traceback(obj):
    return None


def start(nframe=25):
    pass


def stop():
    pass


def get_traced_memory():
    return (0, 0)


def get_traceback_limit():
    return 0


def is_tracing():
    return False


def clear_traces():
    pass


def take_snapshot():
    return Snapshot()


class Snapshot:
    def compare_to(self, old, group_by='lineno', cumulative=False):
        return []
    def statistics(self, group_by=True):
        return []
    def traceback_filter(self, func):
        return Snapshot()
    def filter_traces(self, filters):
        pass


class Frame:
    def __init__(self, filename, lineno):
        self.filename = filename
        self.lineno = lineno


class Statistic:
    def __init__(self, count, size, traceback):
        self.count = count
        self.size = size
        self.traceback = traceback


class Traceback:
    def __init__(self):
        self.frames = []

    def __iter__(self):
        return iter(self.frames)

    def __len__(self):
        return len(self.frames)

    def __getitem__(self, i):
        return self.frames[i]
