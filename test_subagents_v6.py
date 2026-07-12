print('=== contextlib ===')
import contextlib
print('nullcontext:', contextlib.nullcontext(42))

print()
print('=== decimal ===')
import decimal
d = decimal.Decimal('3.14')
print('Decimal:', d)

print()
print('=== fractions ===')
import fractions
f = fractions.Fraction(3, 4)
print('Fraction:', f)
f2 = fractions.Fraction(6, 8)
print('Fraction reduced:', f2)

print()
print('=== random.shuffle ===')
import random
lst = [1, 2, 3, 4, 5]
random.shuffle(lst)
print('shuffled:', lst)

print()
print('=== super() ===')
class A:
    def method(self):
        return 'A'
class B(A):
    def method(self):
        return 'B'
b = B()
print('B.method():', b.method())

print()
print('=== memoryview ===')
mv = memoryview(b'hello')
print('memoryview:', mv)

print()
print('=== delattr ===')
class C:
    def __init__(self):
        self.x = 1
        self.y = 2
obj = C()
delattr(obj, 'x')
print('after delattr, has x:', hasattr(obj, 'x'))
print('after delattr, has y:', hasattr(obj, 'y'))

print()
print('=== @ matmul ===')
class Mat:
    def __matmul__(self, other):
        return 'matmul!'
m = Mat()
print('a @ b:', m @ m)

print()
print('=== ALL TESTS PASSED ===')
