# Test missing features to see what works and what doesn't
import sys

print("=== 1. Test 'with' statement ===")
try:
    with open("/dev/null", "w") as f:
        f.write("x")
    print("with: OK")
except Exception as e:
    print(f"with: FAIL - {e}")

print("=== 2. Test 'del' statement ===")
x = 42
del x
try:
    print(x)
    print("del: FAIL - should have raised NameError")
except NameError:
    print("del: OK")

print("=== 3. Test 'finally' ===")
try:
    pass
finally:
    print("finally: OK")

print("=== 4. Test 'raise...from' ===")
try:
    raise ValueError("inner") from TypeError("outer")
except ValueError as e:
    print(f"raise...from: OK (cause={e.__cause__})")

print("=== 5. Test 'except...as' ===")
try:
    raise ValueError("test")
except ValueError as e:
    print(f"except...as: OK ({e})")

print("=== 6. Test decorators ===")
def deco(f):
    def wrapper():
        return f() + 1
    return wrapper
@deco
def foo():
    return 1
assert foo() == 2
print("decorator: OK")

print("=== 7. Test '__getitem__' on custom class ===")
class MySeq:
    def __getitem__(self, idx):
        return idx * 2
s = MySeq()
r = s[5]
assert r == 10, f"got {r}"
print("__getitem__: OK")

print("=== 8. Test '__setitem__' on custom class ===")
class MyDict:
    def __setitem__(self, key, value):
        self.stored = (key, value)
d = MyDict()
d["hello"] = 42
assert d.stored == ("hello", 42)
print("__setitem__: OK")

print("=== 9. Test '__iter__' on custom class ===")
class MyRange:
    def __init__(self, n):
        self.n = n
    def __iter__(self):
        return MyRangeIter(self.n)
class MyRangeIter:
    def __init__(self, n):
        self.i = 0
        self.n = n
    def __next__(self):
        if self.i >= self.n:
            raise StopIteration
        self.i += 1
        return self.i - 1
r = list(MyRange(3))
assert r == [0, 1, 2]
print("__iter__/__next__: OK")

print("=== 10. Test async/await ===")
async def afunc():
    return 42
print("async def: OK")

print("=== 11. Test matrix multiply ===")
class Mat:
    def __matmul__(self, other):
        return "matmul"
m = Mat()
r = m @ m
assert r == "matmul"
print("matrix multiply: OK")

print("=== 12. Test multiple inheritance ===")
class A:
    def method(self):
        return "A"
class B:
    def method(self):
        return "B"
class C(A, B):
    pass
c = C()
print(f"multiple inheritance MRO: OK (method={c.method()})")

print("\n=== Phase 1 COMPLETE ===")
