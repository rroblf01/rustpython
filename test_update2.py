d = {'a': 1, 'b': 2}
other = {'c': 3}
d.update(other)
print('dict.update with variable: OK')
assert d['c'] == 3
