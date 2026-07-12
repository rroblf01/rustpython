# Bug 1: match as first statement inside function
# This should work: 'def f(): match 1: case _: return ok'
print('START')
def f():
    match 1:
        case _:
            return 'ok'
print('def done')
result = f()
print('result:', result)
print('DONE')
