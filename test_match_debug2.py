def f():
    print('start f')
    x = 1
    print('before match')
    match x:
        case _:
            print('matched wildcard')
    print('after match')
print('calling f')
f()
print('DONE')
