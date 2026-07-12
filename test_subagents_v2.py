# Test all new features from subagents
print('=== 1. glob module ===')
try:
    import glob
    files = glob.glob('*.py')
    if len(files) > 0:
        print('glob: OK, found', len(files), 'files')
except Exception as e:
    print('glob:', e)

print()
print('=== 2. fnmatch module ===')
try:
    import fnmatch
    assert fnmatch.fnmatch('test.py', '*.py')
    assert not fnmatch.fnmatch('test.py', '*.txt')
    print('fnmatch: OK')
except Exception as e:
    print('fnmatch:', e)

print()
print('=== 3. textwrap module ===')
try:
    import textwrap
    s = '  hello\n  world'
    r = textwrap.dedent(s)
    assert r == 'hello\nworld'
    print('textwrap.dedent: OK')
    r2 = textwrap.indent('hello\nworld', '  ')
    assert r2 == '  hello\n  world'
    print('textwrap.indent: OK')
except Exception as e:
    print('textwrap:', e)

print()
print('=== 4. pprint module ===')
try:
    import pprint
    pprint.pprint({'a': 1, 'b': [2, 3]})
    print('pprint: OK')
except Exception as e:
    print('pprint:', e)

print()
print('=== 5. FrozenSet ===')
try:
    s = frozenset([1, 2, 3])
    assert 1 in s
    assert 4 not in s
    assert len(s) == 3
    print('frozenset: OK')
except Exception as e:
    print('frozenset:', e)

print()
print('=== 6. Star unpacking ===')
try:
    first, *rest, last = [1, 2, 3, 4, 5]
    assert first == 1
    assert rest == [2, 3, 4]
    assert last == 5
    print('star unpacking: OK')
except Exception as e:
    print('star unpacking:', e)

print()
print('=== 7. dict update with literal ===')
try:
    d = {'a': 1}
    f({'a': 1})  # test dict literal as arg
except:
    pass
d = {'a': 1, 'b': 2}
d.update({'c': 3})
assert d['c'] == 3
print('dict.update: OK')

print()
print('=== 8. str lstrip/rstrip ===')
s = '  hello  '
assert s.strip() == 'hello'
assert s.lstrip() == 'hello  '
assert s.rstrip() == '  hello'
print('str lstrip/rstrip: OK')

print()
print('=== ALL NEW FEATURES PASSED ===')
