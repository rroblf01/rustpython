def f(x):
    match x:
        case 1:
            return 'one'
        case _:
            return 'other'
print('f(1):', f(1))
print('DONE')
