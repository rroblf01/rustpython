# Test dict.update in isolation
d = {'a': 1, 'b': 2}
d.update({'c': 3})
assert 'c' in d
assert d['c'] == 3
print('dict.update: OK')

# Test dict.get
assert d.get('a') == 1
assert d.get('x', 99) == 99
print('dict.get: OK')

# Test dict.pop with default
v = d.pop('x', None)
assert v is None
print('dict.pop with default: OK')

# Test list.sort
l = [3, 1, 2]
l.sort()
assert l == [1, 2, 3]
print('list.sort: OK')

# Test list.index with value equality
l = [1, 2, 3]
assert l.index(2) == 1
print('list.index: OK')

# Test list.count with value equality
l = [1, 2, 1, 3]
assert l.count(1) == 2
print('list.count: OK')

# Test format spec
x = 42
s = f"{x:>10}"
assert len(s) == 10
print('format >10: OK')

pi = 3.14159
s2 = f"{pi:.2f}"
assert s2 == '3.14'
print('format .2f: OK')

print()
print('ALL DICT/LIST/FORMAT TESTS PASSED')
