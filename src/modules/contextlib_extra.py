from functools import wraps


class ContextDecorator:
    def _recreate_cm(self):
        return self

    def __call__(self, func):
        @wraps(func)
        def inner(*args, **kwargs):
            with self._recreate_cm():
                return func(*args, **kwargs)
        return inner
