# Bug 3: dict.update with dict literal
print('START')
d = {'a': 1, 'b': 2}
print('before:', d)
d.update({'c': 3})
print('after:', d)
print('DONE')
