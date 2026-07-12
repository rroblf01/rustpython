# Does match as ONLY statement work?
print('A')
def f():
    match 1:
        case _:
            return 'ok'
print('B')
result = f()
print('C result:', result)
print('DONE')
