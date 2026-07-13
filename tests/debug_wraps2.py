import functools
print("functools loaded")
print("wraps:", functools.wraps)
print("type:", type(functools.wraps))
# Manually test wraps
def dummy():
    pass
deco = functools.wraps(dummy)
print("deco type:", type(deco))
print("deco:", repr(deco))
