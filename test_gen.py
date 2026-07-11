# Test: can we access __next__ on generator?
def gen():
    yield 1

g = gen()
try:
    n = g.__next__
    print("__next__ attr found:", n)
except AttributeError as e:
    print("AttributeError:", e)

try:
    has = hasattr(g, "__next__")
    print("hasattr __next__:", has)
except Exception as e:
    print("hasattr error:", e)
