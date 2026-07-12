def f(x):
    print('x is', x)
    y = x
    match y:
        case 1:
            return 'one'
        case _:
            return 'other'
print('f(1):', f(1))
print('DONE')
