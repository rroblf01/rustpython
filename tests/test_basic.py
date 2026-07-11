# Basic RustPython tests
print("=== Arithmetic ===")
assert 1 + 1 == 2
assert 3 * 4 == 12
assert 2 ** 3 == 8
print("OK")

print("=== Booleans ===")
assert True and True
assert True or False
assert not False
print("OK")

print("=== Lists ===")
lst = [1, 2, 3]
lst.append(4)
assert lst.pop() == 4
lst.reverse()
assert lst == [3, 2, 1]
print("OK")

print("=== Strings ===")
assert "hello".upper() == "HELLO"
assert "hello".replace("l", "x") == "hexxo"
print("OK")

print("=== Dicts ===")
d = {"a": 1}
assert d.get("a") == 1
assert d.get("x", 99) == 99
print("OK")

print("=== Functions ===")
def f(x):
    return x * 2
assert f(5) == 10
print("OK")

print("=== Lambdas ===")
double = lambda x: x * 2
assert double(3) == 6
print("OK")

print("=== Classes ===")
class Counter:
    count = 0
    def inc(self):
        self.count = self.count + 1
        return self.count
c = Counter()
assert c.inc() == 1
assert c.inc() == 2
print("OK")

print("=== isinstance ===")
assert isinstance(1, int)
assert isinstance("hello", str)
assert isinstance([], list)
assert isinstance(True, bool)
assert isinstance(1.5, float)
assert isinstance({}, dict)
assert isinstance((1,), tuple)
assert isinstance({1,2}, set)
assert isinstance(b"hi", bytes)
class MyClass: pass
obj = MyClass()
assert isinstance(obj, MyClass)
assert not isinstance(1, MyClass)
print("OK")

print("=== Comprehensions ===")
assert [x * x for x in [1, 2, 3]] == [1, 4, 9]
print("OK")

print("=== Generators ===")
def gen():
    yield 1
    yield 2
g = gen()
assert next(g) == 1
assert next(g) == 2
try:
    next(g)
    assert False
except StopIteration:
    pass
print("OK")

print("=== GenExpr ===")
g = (x * x for x in [1, 2, 3])
assert list(g) == [1, 4, 9]
print("OK")

print("=== Match ===")
y = 2
match y:
    case 1: r = "one"
    case 2: r = "two"
    case _: r = "other"
assert r == "two"
print("OK")

print("=== __str__ ===")
class Person:
    def __str__(self):
        return "Person"
assert str(Person()) == "Person"
print("OK")

print("=== __len__ ===")
class MyList:
    def __len__(self):
        return 42
assert len(MyList()) == 42
print("OK")

print("=== Try/except ===")
try:
    raise ValueError("test")
except ValueError:
    pass
print("OK")

print("=== Docstrings ===")
def func():
    "doc"
    pass
assert func.__doc__ == "doc"
print("OK")

print("=== Walrus ===")
if (x := 42) > 10:
    assert x == 42
print("OK")

print("=== *args ===")
def va(*args):
    return args
assert va() == ()
assert va(1) == (1,)
assert va(1, 2, 3) == (1, 2, 3)
print("OK")

print("=== **kwargs ===")
def kw(**kwargs):
    return kwargs
assert kw() == {}
r = kw(a=1, b=2)
assert r.get("a") == 1
assert r.get("b") == 2
print("OK")

print("=== Defaults ===")
def defaults(a, b=10, c=20):
    return a + b + c
assert defaults(1) == 31
assert defaults(1, 2) == 23
assert defaults(1, 2, 3) == 6
print("OK")

print("=== Mixed *args/**kwargs ===")
def mixed(a, *args, **kwargs):
    return (a, args, kwargs)
r = mixed(1, 2, 3, x=4)
assert r[0] == 1
assert r[1] == (2, 3)
assert r[2].get("x") == 4
print("OK")

print("ALL TESTS PASSED!")
