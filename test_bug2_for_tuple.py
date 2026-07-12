# Bug 2: For loop with tuple unpacking
print('START')

# Test 1: for (a, b) in ...  (with parens)
items1 = [(1, 'a'), (2, 'b')]
for (a, b) in items1:
    print('t1:', a, b)

# Test 2: for a, b in ...  (without parens)  
items2 = [(3, 'c'), (4, 'd')]
for a, b in items2:
    print('t2:', a, b)

print('DONE')
