# Test 1: Match/case basic
print("before match")
x = 1
match x:
    case 1:
        print("matched 1")
    case _:
        print("matched wildcard")
print("after match")
