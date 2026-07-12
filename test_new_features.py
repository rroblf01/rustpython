# Test all new subagent features (fixed)
print('=== bisect ===')
import bisect
a = [1, 3, 5, 7, 9]
i = bisect.bisect_left(a, 6)
assert i == 3
print('bisect: OK')

print()
print('=== heapq ===')
import heapq
h = [5, 3, 1, 4, 2]
heapq.heapify(h)
x = heapq.heappop(h)
assert x == 1
print('heapq: OK')

print()
print('=== enum ===')
import enum
class Color(enum.Enum):
    RED = 1
    GREEN = 2
print('Color.RED:', Color.RED)
print('Color.GREEN:', Color.GREEN)
assert Color.RED == 1
print('enum: OK')

print()
print('=== dict methods ===')
d = {'a': 1, 'b': 2}
assert d.get('a') == 1
assert d.get('x', 99) == 99
v = d.pop('a')
assert v == 1
assert 'a' not in d
d.update({'c': 3})
assert d.get('c') == 3
print('dict methods: OK')

print()
print('=== list methods ===')
l = [3, 1, 4, 1, 5, 9]
assert l.index(4) == 2
assert l.count(1) == 2
l.sort()
assert l == [1, 1, 3, 4, 5, 9]
print('list methods: OK')

print()
print('=== format spec ===')
x = 42
s = f"{x:>10}"
assert len(s) == 10
assert s.rstrip() == '42'
print('format >10:', repr(s))

pi = 3.14159
s2 = f"{pi:.2f}"
print('format .2f:', s2)

s3 = f"{x:x}"
print('format hex:', s3)

print()
print('=== ALL TESTS PASSED ===')
