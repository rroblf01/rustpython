WRAPPER_ASSIGNMENTS = ('__module__', '__name__', '__qualname__', '__doc__',
                       '__annotations__')
WRAPPER_UPDATES = ('__dict__',)


def update_wrapper(wrapper, wrapped, assigned=WRAPPER_ASSIGNMENTS,
                    updated=WRAPPER_UPDATES):
    """Real functools.update_wrapper, replacing the previous native
    `update_wrapper`/`wraps` (src/modules/data/functools.rs) that hardcoded
    a fixed attribute list and completely ignored the `assigned`/`updated`
    parameters — `functools.update_wrapper(wrapper, f, (), ())` (real
    trigger: test_functools.py's TestUpdateWrapper.test_no_update) still
    copied __name__/__doc__/etc. from `wrapped` onto `wrapper` even though
    an empty tuple was passed for `assigned`.
    """
    for attr in assigned:
        try:
            value = getattr(wrapped, attr)
        except AttributeError:
            pass
        else:
            setattr(wrapper, attr, value)
    for attr in updated:
        getattr(wrapper, attr).update(getattr(wrapped, attr, {}))
    wrapper.__wrapped__ = wrapped
    return wrapper


def wraps(wrapped, assigned=WRAPPER_ASSIGNMENTS, updated=WRAPPER_UPDATES):
    def decorator(wrapper):
        return update_wrapper(wrapper, wrapped, assigned, updated)
    return decorator


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


_NOT_FOUND = object()


class cached_property:
    """Real functools.cached_property, replacing the previous native
    `cached_property` stub (`src/modules/data/functools.rs`) that just
    returned the wrapped function UNCHANGED — a bare `def bar(self): ...`
    decorated with `@functools.cached_property` was never replaced by a
    descriptor at all, so `instance.bar` returned an ordinary bound method
    instead of invoking (and caching) the computed value. Real trigger:
    `Lib/platform.py`'s `uname_result.processor` (a `@cached_property`),
    used by `platform.uname()`/`platform.platform()` — accessing
    `.processor` returned `<bound method uname_result.processor ...>`
    itself, which then broke any code trying to use it as a plain value
    (e.g. `Lib/platform.py`'s own `_platform()`, called from `ssl.py` at
    import time: `TypeError: object of type 'method' has no len()`).
    Matches real CPython's own `functools.cached_property` semantics:
    a non-data descriptor (no `__set__`/`__delete__`) that computes the
    value once and caches it directly into `instance.__dict__` under the
    same name, so subsequent attribute lookups find the cached value
    directly (without even going through `__get__` again).
    """

    def __init__(self, func):
        self.func = func
        self.attrname = None
        self.__doc__ = getattr(func, "__doc__", None)

    def __set_name__(self, owner, name):
        if self.attrname is None:
            self.attrname = name
        elif name != self.attrname:
            raise TypeError(
                "Cannot assign the same cached_property to two different "
                f"names ({self.attrname!r} and {name!r})."
            )

    def __get__(self, instance, owner=None):
        if instance is None:
            return self
        if self.attrname is None:
            raise TypeError(
                "Cannot use cached_property instance without calling "
                "__set_name__ on it."
            )
        try:
            cache = instance.__dict__
        except AttributeError:
            raise TypeError(
                f"No '__dict__' attribute on {type(instance).__name__!r} "
                f"instance to cache {self.attrname!r} property."
            ) from None
        val = cache.get(self.attrname, _NOT_FOUND)
        if val is _NOT_FOUND:
            val = self.func(instance)
            try:
                cache[self.attrname] = val
            except TypeError:
                raise TypeError(
                    f"The '__dict__' attribute on {type(instance).__name__!r} "
                    f"instance does not support item assignment for caching "
                    f"{self.attrname!r} property."
                ) from None
        return val

    __class_getitem__ = classmethod(lambda cls, item: cls)
