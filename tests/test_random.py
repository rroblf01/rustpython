# Test random module
import random

# Basic random functions
r = random.random()
assert 0.0 <= r < 1.0, f"random() out of range: {r}"

ri = random.randint(1, 10)
assert 1 <= ri <= 10, f"randint out of range: {ri}"

rc = random.choice([1, 2, 3])
assert rc in [1, 2, 3]

print("OK")
