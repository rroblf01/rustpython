def f(x):
    print('before match')
    match x:
        case 1:
            return 'one'
        case _:
            return 'other'
    return 'end'
print('f(1):', f(1))
print('DONE')
