# Test __slots__ implementation
# Test 1: Basic __slots__ restriction
class A:
    __slots__ = ('x', 'y')

a = A()
print("Test 1: Basic slots creation")
a.x = 42
print(f"  a.x = {a.x}")  # Should work
a.y = "hello"
print(f"  a.y = {a.y}")  # Should work

try:
    a.z = "forbidden"
    print("  ERROR: Should have raised AttributeError for 'z'")
except AttributeError as e:
    print(f"  OK: Correctly blocked 'z': {e}")

# Test 2: __slots__ as a list
class B:
    __slots__ = ['a', 'b']

b = B()
print("\nTest 2: List-based slots")
b.a = 1
b.b = 2
print(f"  b.a = {b.a}, b.b = {b.b}")

try:
    b.c = 3
    print("  ERROR: Should have raised AttributeError for 'c'")
except AttributeError as e:
    print(f"  OK: Correctly blocked 'c': {e}")

# Test 3: __slots__ as a string (single slot)
class C:
    __slots__ = 'only_one'

c = C()
print("\nTest 3: String-based slot")
c.only_one = 1
print(f"  c.only_one = {c.only_one}")

try:
    c.other = 2
    print("  ERROR: Should have raised AttributeError for 'other'")
except AttributeError as e:
    print(f"  OK: Correctly blocked 'other': {e}")

# Test 4: Inherited slots
class D(A):  # A has __slots__ = ('x', 'y')
    __slots__ = ('z',)

d = D()
print("\nTest 4: Inherited slots")
d.x = 10  # From A
d.y = 20  # From A
d.z = 30  # From D
print(f"  d.x = {d.x}, d.y = {d.y}, d.z = {d.z}")

try:
    d.w = 99
    print("  ERROR: Should have raised AttributeError for 'w'")
except AttributeError as e:
    print(f"  OK: Correctly blocked 'w': {e}")

# Test 5: Class without __slots__ (should work normally)
class E:
    pass

e = E()
print("\nTest 5: No __slots__ (normal behavior)")
e.anything = 123
print(f"  e.anything = {e.anything}")

# Test 6: Methods still accessible with __slots__
class F:
    __slots__ = ('x',)
    def method(self):
        return 42

f = F()
print("\nTest 6: Methods work with __slots__")
print(f"  f.method() = {f.method()}")

# Test 7: Deleting attributes with __slots__
class G:
    __slots__ = ('x', 'y')

g = G()
g.x = 1
g.y = 2
print("\nTest 7: Attribute deletion with __slots__")
del g.x
try:
    print(f"  g.x = {g.x}")  # Should raise AttributeError after deletion
except AttributeError:
    print("  OK: Deleted attribute correctly")

try:
    del g.z
    print("  ERROR: Should have raised AttributeError for deleting non-slot 'z'")
except AttributeError as e:
    print(f"  OK: Correctly blocked deletion of 'z': {e}")

print("\nAll tests passed!")
