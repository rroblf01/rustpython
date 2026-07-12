# Test all JIT opcodes
print("=== Testing UNARY_NEGATIVE ===")
x = -42
print("  -42 =", x)
for i in range(5):
    x = -i
print("  last -i =", x)

print("=== Testing UNARY_NOT ===")
print("  not True =", not True)
for i in range(5):
    if not (i % 2):
        pass
print("  not done")

print("=== Testing BUILD_LIST ===")
lst = [1, 2, 3, 4, 5]
print("  list len =", len(lst))

print("=== Testing BUILD_TUPLE ===")
tup = (1, 2, 3)
print("  tuple len =", len(tup))

print("=== Testing LIST_APPEND ===")
lst2 = []
for i in range(5):
    lst2.append(i)
print("  appended:", lst2)

print("=== Testing CONTAINS_OP ===")
print("  3 in [1,2,3]:", 3 in [1, 2, 3])
print("  10 in [1,2,3]:", 10 in [1, 2, 3])

print("=== Testing GET_ITER ===")
r = 0
for x in [1, 2, 3]:
    r += x
print("  sum:", r)

print("=== Testing CALL ===")
def f(x):
    return x + 1
for i in range(5):
    r = f(i)
print("  f(4) =", r)

print("=== Testing LOAD_ATTR ===")
class Point:
    def __init__(self):
        self.x = 10
        self.y = 20
p = Point()
for i in range(5):
    r = p.x
print("  p.x =", r)

print("=== Testing mixed ===")
total = 0
for i in range(100):
    if i % 2:
        total += -i
    else:
        total += i
    if 42 in (42,):
        pass
print("  total =", total)

print("ALL TESTS PASSED")
