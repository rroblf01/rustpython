# Debug test: Check if __slots__ is in the type dict
class A:
    __slots__ = ('x', 'y')

# Now test the instance
a = A()
print("setting a.x = 42...")
a.x = 42
print("a.x =", a.x)

print("setting a.z = 'should fail'...")
try:
    a.z = "should fail"
    print("ERROR: a.z =", a.z, "(should have been blocked)")
except AttributeError as e:
    print("OK: blocked a.z:", e)

# Also test class-level __slots__ access
print("\nClass-level access:")
try:
    s = A.__slots__
    print("A.__slots__ =", s)
except AttributeError as e:
    print("A.__slots__ access error:", e)

print("\nDone")
