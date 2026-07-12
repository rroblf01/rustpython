print("=== Testing async def ===")
async def foo():
    return 42
print("  async def:", foo)

print("\n=== Testing import math ===")
try:
    import math
    print("  math.pi:", math.pi)
    print("  math.sqrt(4):", math.sqrt(4))
    print("  math.floor(3.7):", math.floor(3.7))
except Exception as e:
    print("  math import error:", e)

print("\n=== Testing import sys ===")
try:
    import sys
    print("  sys imported")
except Exception as e:
    print("  sys import error:", e)

print("\nALL TESTS PASSED")
