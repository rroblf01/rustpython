def f(x):
    print('match called with', x)
    match x:
        case 1:
            return 'one'
        case _:
            return 'other'
    return 'end'
print('first call')
print('f(1):', f(1))
print('second call')
print('f(5):', f(5))
print('DONE')
