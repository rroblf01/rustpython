"""iscoroutinefunction module - placeholder."""

def iscoroutinefunction(func):
    return hasattr(func, '__code__') and func.__code__.co_flags & 0x80
