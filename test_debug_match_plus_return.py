# Does match + another statement work inside function (like test_match_minimal)?
print('A')
def f():
    match 1:
        case _:
            return 'ok'
    return 'end'
print('B')
result = f()
print('C result:', result)
print('DONE')
