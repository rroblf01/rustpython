"""Test: @staticmethod + @property interaction in RustPython - detailed"""

# Test 1: Basic property access
print("=== Test 1: Basic @property ===")
class A:
    @property
    def x(self):
        return 42

a = A()
print(f"a.x = {a.x}")
assert a.x == 42, f"Test 1 failed: a.x should be 42, got {a.x}"
print("PASS")

# Test 2: Basic staticmethod access on class
print("\n=== Test 2: Basic @staticmethod access on class ===")
class B:
    @staticmethod
    def greet(name):
        return f"Hello, {name}!"

print(f"B.greet('world') = {B.greet('world')}")
assert B.greet('world') == "Hello, world!", f"Test 2 failed"
print("PASS")

# Test 3: Access staticmethod from property getter
print("\n=== Test 3: Staticmethod from property getter ===")
class _Constants:
    @staticmethod
    def _is_private(ip):
        return (ip & 0xFF000000) == 0x0A000000

class IPv4Address:
    def __init__(self, ip):
        self._ip = ip
    
    @property
    def is_private(self):
        return _Constants._is_private(self._ip)

a = IPv4Address(0x0A000001)
print(f"a.is_private = {a.is_private}")
assert a.is_private is True, f"Test 3 failed: expected True, got {a.is_private}"
print("PASS")

# Test 4: Direct staticmethod call (regression check)
print("\n=== Test 4: Direct staticmethod call ===")
result = _Constants._is_private(0x0A000001)
print(f"Direct: {result}")
assert result is True, f"Test 4 failed"
print("PASS")

# Test 5: Classmethod still works
print("\n=== Test 5: @classmethod regression ===")
class C:
    @classmethod
    def identity(cls, x):
        return x

result = C.identity(42)
print(f"C.identity(42) = {result}")
assert result == 42, f"Test 5 failed: {result}"
print("PASS")

print("\n=== ALL TESTS PASSED ===")
