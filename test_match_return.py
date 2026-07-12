def f():
    match 1:
        case 1:
            return 'one'
        case _:
            return 'other'
    return 'end'
print('f():', f())
print('DONE')
