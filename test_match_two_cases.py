def f(x):
    match x:
        case 1:
            return 'one'
        case _:
            return 'other'
    return 'end'
print('f(1):', f(1))
print('f(99):', f(99))
print('DONE')
