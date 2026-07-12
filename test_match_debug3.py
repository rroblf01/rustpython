# Debug: which part fails?
print('A')
def f():
    print('inside f before match')
    match 1:
        case _:
            print('inside match case')
            return 'ok'
print('B')
result = f()
print('C - result:', result)
print('DONE')
