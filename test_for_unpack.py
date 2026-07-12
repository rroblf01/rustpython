# Test 1: basic tuple unpacking
for a, b in [(1,2)]:
    print(a)
    print(b)

# Test 2: trailing comma after tuple target
for c, in [(1,)]:
    print(c)

# Test 3: star unpacking in for
# for d, *e in [(1,2,3)]:
#     print(d)
#     print(e)

# Test 4: nested tuple unpacking
for (x, y) in [(1,2)]:
    print(x)
    print(y)
