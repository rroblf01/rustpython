print("=== Testing IS_OP ===")
print("  None is None:", None is None)
print("  1 is 1:", 1 is 1)

print("\n=== Testing UNARY_INVERT ===")
print("  ~0 =", ~0)
print("  ~5 =", ~5)

print("\n=== Testing BUILD_SET ===")
s = {1, 2, 3}
print("  set len:", len(s))
print("  1 in set:", 1 in s)

print("\n=== Testing STORE_SUBSCR ===")
d = {}
d["key"] = 42
print("  d[key]:", d["key"])

print("\n=== Testing POP_JUMP_IF_TRUE ===")
for i in range(3):
    if i:
        print("  true:", i)

print("\n=== Testing POP_JUMP_IF_NONE ===")
x = None
if x is None:
    print("  x is None: True")

print("\n=== Testing context managers ===")
try:
    try:
        raise ValueError("test")
    except ValueError:
        print("  caught ValueError")
except:
    print("  outer except")

print("\nALL TESTS PASSED")
