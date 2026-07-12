import weakref
import copy
import collections

# Test weakref
o = object()
r = weakref.ref(o)
assert r() is o
print('weakref: OK')

# Test copy
x = [1, 2, [3, 4]]
y = copy.deepcopy(x)
x[2][0] = 99
assert y[2][0] == 3
print('copy.deepcopy: OK')

z = copy.copy([1, 2, 3])
assert z == [1, 2, 3]
print('copy.copy: OK')

# Test f-string format specs
x_val = 42
s1 = f"{x_val!r}"
assert s1 == '42', f"expected '42', got {s1!r}"
print('fstring !r: OK')

s2 = f"{x_val!s}"
assert s2 == '42'
print('fstring !s: OK')

s3 = f"{x_val:>10}"
assert len(s3) == 10
print('fstring format spec: OK')

s4 = f"{x_val!r:10}"
assert s4 == '42'
print('fstring !r:10: OK')

print()
print('=== ALL TESTS PASSED ===')
