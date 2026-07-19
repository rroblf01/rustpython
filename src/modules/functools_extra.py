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


def lru_cache(maxsize=128):
    if callable(maxsize):
        func = maxsize
        return _lru_cache_wrapper(func, 128)

    def decorator(func):
        return _lru_cache_wrapper(func, maxsize)
    return decorator


def cache(func):
    return _lru_cache_wrapper(func, None)
