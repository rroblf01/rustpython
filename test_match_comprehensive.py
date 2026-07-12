# Test various match/case scenarios
print("=== Test 1: basic match ===")
print("before match")
x = 1
match x:
    case 1:
        print("matched 1")
print("after match 1")

print("=== Test 2: match wildcard ===")
print("before wildcard")
match x:
    case _:
        print("matched wildcard")
print("after wildcard")

print("=== Test 3: match no match ===")
x = 99
print("before nomatch")
match x:
    case 1:
        print("should not print")
print("after nomatch")
