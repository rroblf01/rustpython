# Test: match + pass (two statements, but pass is a no-op)
print('A')
def f():
    match 1:
        case _:
            return 'ok'
    pass
print('B')
result = f()
print('C result:', result)
print('DONE')
