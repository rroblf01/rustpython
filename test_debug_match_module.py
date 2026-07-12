# Test: does match work as only statement in module level (not in function)?
print('A')
match 1:
    case _:
        print('matched!')
print('B')
