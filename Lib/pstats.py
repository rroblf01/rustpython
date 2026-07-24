"""Statistics object for profiler output."""

import os


class Stats:
    """Simple statistics object for profiler results."""

    def __init__(self, *args, stream=None):
        self.stream = stream or __import__('sys').stdout
        self.files = []
        self.func_list = []
        self.stats = {}
        for arg in args:
            if isinstance(arg, str):
                self.files.append(arg)
                self.load_stats(arg)

    def load_stats(self, arg):
        """Load stats from a file."""
        import marshal
        try:
            with open(arg, 'rb') as f:
                self.stats = marshal.load(f)
        except Exception:
            self.stats = {}

    def sort_stats(self, *keys):
        return self

    def reverse_order(self):
        return self

    def print_stats(self, *amount):
        self.stream.write("Profile statistics\n")
        for func, (cc, nc, tt, ct, callers) in self.stats.items():
            self.stream.write(f"   {func}: {nc} calls, {tt:.3f} total\n")

    def print_callers(self, *amount):
        self.stream.write("Function callers:\n")

    def print_callees(self, *amount):
        self.stream.write("Function callees:\n")

    def add(self, *args):
        return self

    def strip_dirs(self):
        return self

    def dump_stats(self, filename):
        import marshal
        with open(filename, 'wb') as f:
            marshal.dump(self.stats, f)

    def get_stats_profile(self):
        import collections
        StatsProfile = collections.namedtuple('StatsProfile', ['func_profiles'])
        return StatsProfile(func_profiles={})
