# Edge case tests for match/case

print("=== Test: match with guard ===")
x = 2
match x:
    case n if n > 1:
        print("greater than 1:", n)
    case _:
        print("other")
print("after guard test")

print()
print("=== Test: match sequence pattern ===")
y = [1, 2]
match y:
    case [a, b]:
        print("sequence matched:", a, b)
    case _:
        print("no match")
print("after sequence test")

print()
print("=== Test: match with nested match ===")
val = 10
match val:
    case 1:
        print("one")
        match val:
            case 2:
                print("nope")
            case _:
                print("nested wildcard")
    case 10:
        print("ten")
    case _:
        print("other")
print("after nested match test")
