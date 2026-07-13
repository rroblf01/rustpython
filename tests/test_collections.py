# Test collections module
from collections import deque, Counter, defaultdict

# deque - basic
d = deque([1, 2, 3])
d.append(4)
assert len(d) == 4

# Counter - basic
c = Counter("aabbc")
assert c["a"] == 2
assert c["b"] == 2
assert c["c"] == 1
assert c["x"] == 0

# defaultdict - basic
dd = defaultdict(int)
assert dd["x"] == 0  # default value for missing key

print("OK")
