# Test custom iterable with __iter__ and __next__
class MyIter:
    def __init__(self, n):
        self.n = n
        self.i = 0
    def __iter__(self):
        return self
    def __next__(self):
        if self.i >= self.n:
            raise StopIteration
        self.i += 1
        return self.i

import dis
print("=== Bytecode of __next__ ===")
dis.dis(MyIter.__next__)
print()

# Test just __iter__ first
print("test __iter__:")
try:
    it = MyIter(3).__iter__()
    print("  __iter__() OK, got:", it)
except Exception as e:
    print("  __iter__() FAIL:", e)

# Test just __next__ on fresh instance
print("test __next__ on fresh instance:")
try:
    obj = MyIter(3)
    print("  instance created, type:", type(obj))
    print("  n =", obj.n, ", i =", obj.i)
    result = obj.__next__()
    print("  __next__() returned:", result)
    print("  now i =", obj.i)
except Exception as e:
    print("  __next__() FAIL:", e)
