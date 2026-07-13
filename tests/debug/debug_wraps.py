import functools
from functools import wraps

def decorator(f):
    @wraps(f)
    def wrapper(x):
        return f(x)
    return wrapper

@decorator
def my_func(x):
    return x + 1

result = my_func(10)
print("result:", result)
assert result == 11, "wraps decorated function failed"
assert my_func.__name__ == "my_func", "wraps __name__ failed"
print("OK")
