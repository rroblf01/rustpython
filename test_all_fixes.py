# Comprehensive tests for all three fixes

print("=== Test: Match/case with multiple cases ===")
x = 3
print("before match")
match x:
    case 1:
        print("one")
    case 2:
        print("two")
    case 3:
        print("three")
    case _:
        print("other")
print("after match")

print()
print("=== Test: Match/case - no match falls through ===")
x = 99
print("before")
match x:
    case 1:
        print("one")
    case 2:
        print("two")
print("after - should see this even without wildcard")

print()
print("=== Test: For loop with tuple unpacking ===")
total = 0
for a, b in [(10, 20), (30, 40), (50, 60)]:
    total = total + a + b
print("total =", total)

print()
print("=== Test: For loop with single target still works ===")
for x in [1, 2, 3]:
    pass
print("simple for works, x =", x)

print()
print("=== Test: Raise from cause ===")
try:
    raise ValueError("inner") from ValueError("outer")
except ValueError as e:
    print("Caught ValueError as expected")

print()
print("All tests passed!")
