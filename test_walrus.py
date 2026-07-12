# Test 1: Walrus operator in if condition with comparison
x = 5
if (y := x + 1) > 0:
    print("walrus works:", y)
else:
    print("walrus failed")

# Test 2: Walrus operator in if condition without comparison (just truthiness)
if (z := x - 10):
    print("z is truthy:", z)
else:
    print("z is falsy:", z)
