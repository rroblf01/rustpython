"""
profile.py — Python profiling interface.

Delegates to the native RustPython profile/cProfile module when available.
"""
import sys

# Try to use the native profile module (provided by RustPython)
_profile = sys.modules.get("profile")
if _profile is None:
    # Fallback stub module
    class _ProfileStub:
        def run(self, cmd, globals=None, locals=None):
            exec(cmd, globals or {}, locals or {})

        def runctx(self, cmd, globals, locals):
            exec(cmd, globals, locals)

        class Profile:
            def __init__(self, *args, **kwargs):
                pass
            def enable(self):
                pass
            def disable(self):
                pass
            def create_stats(self):
                pass
            def print_stats(self, *args):
                pass
            def dump_stats(self, filename):
                pass

    _profile = _ProfileStub()

run = _profile.run
runctx = _profile.runctx
Profile = _profile.Profile


def runcall(func, *args, **kwargs):
    """Profile a single callable."""
    return func(*args, **kwargs)
