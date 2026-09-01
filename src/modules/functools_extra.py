class _lru_cache_wrapper:
    def __init__(self, func, maxsize):
        self.__wrapped__ = func
        self.maxsize = maxsize
        self._cache = {}
        self._hits = 0
        self._misses = 0

    def __call__(self, *args, **kwargs):
        key = (args, tuple(sorted(kwargs.items())) if kwargs else ())
        if key in self._cache:
            self._hits += 1
            return self._cache[key]
        self._misses += 1
        result = self.__wrapped__(*args, **kwargs)
        if self.maxsize is None or len(self._cache) < self.maxsize:
            self._cache[key] = result
        return result

    def __get__(self, instance, owner):
        if instance is None:
            return self
        return _bound_cache_wrapper(self, instance)

    def cache_clear(self):
        self._cache.clear()
        self._hits = 0
        self._misses = 0

    def cache_info(self):
        return (self._hits, self._misses, self.maxsize, len(self._cache))


class _bound_cache_wrapper:
    def __init__(self, wrapper, instance):
        self._wrapper = wrapper
        self._instance = instance

    def __call__(self, *args, **kwargs):
        return self._wrapper(self._instance, *args, **kwargs)

    def cache_clear(self):
        self._wrapper.cache_clear()

    def cache_info(self):
        return self._wrapper.cache_info()


def lru_cache(maxsize=128, typed=False):
    # typed is ignored (RustPython doesn't have separate typed caches, but
    # signature must accept it for CPython compat: lru_cache(maxsize, typed)
    # and lru_cache(typed=True) etc., as used in annotationlib).
    # Handle @lru_cache without parens: @lru_cache -> maxsize is func
    if callable(maxsize) and not isinstance(maxsize, bool) and typed is False:
        # Check if maxsize is actually a function (callable and not int/None/bool)
        # lru_cache can be called as @lru_cache or @lru_cache() or @lru_cache(maxsize=...)
        # When used as @lru_cache without args, first arg is the function
        try:
            # If maxsize is callable and not an int, treat as func
            if hasattr(maxsize, '__call__') and not isinstance(maxsize, (int, type(None))):
                func = maxsize
                return _lru_cache_wrapper(func, 128)
        except:
            pass

    def decorator(func):
        return _lru_cache_wrapper(func, maxsize if isinstance(maxsize, int) or maxsize is None else 128)
    # Support @lru_cache(...) with typed
    # Also support lru_cache(maxsize=..., typed=...) as function call returning decorator
    # When called as lru_cache(typed=True) without maxsize, maxsize will be 128 default
    return decorator


def cache(func):
    return _lru_cache_wrapper(func, None)
