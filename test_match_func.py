print('A')
def f(x):
    match x:
        case 1:
            return 'one'
    return 'other'
print('f(1):', f(1))
print('f(5):', f(5))
print('DONE')
