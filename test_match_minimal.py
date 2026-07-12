def f(x):
    match x:
        case _:
            return 'ok'
    return 'end'
print('f(42):', f(42))
print('DONE')
