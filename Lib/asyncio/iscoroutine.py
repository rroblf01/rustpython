"""iscoroutine module - placeholder for RustPython."""

def iscoroutine(obj):
    return hasattr(obj, '__await__') or isinstance(obj, type(lambda: (yield)))
