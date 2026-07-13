from functools import wraps

# Step 1: wraps returns a decorator
def inner():
    pass

decorator_fn = wraps(inner)
print("deco type:", type(decorator_fn))
print("deco:", repr(decorator_fn))

# Step 2: apply the decorator to a function
def target(x):
    return x + 1

try:
    result = decorator_fn(target)
    print("result type:", type(result))
    print("result:", repr(result))
except Exception as e:
    print("Error:", e)
