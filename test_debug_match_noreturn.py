# Test: match as only stmt, no return inside
print('A')
def f():
    match 1:
        case _:
            pass
print('B')
f()
print('C')
print('DONE')
