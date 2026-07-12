# Test dict literal in various contexts
d = {'c': 3}
print('direct:', d)

# test as function arg
def f(x):
    print('f received:', x)
f({'a': 1})

# test update with inline literal  
d2 = {'a': 1, 'b': 2}
x = {'c': 3}
d2.update(x)
print('update with var:', d2)

# test method call with inline literal
d3 = {'a': 1, 'b': 2}
print('before:', d3)
d3.update({'c': 3})
print('after:', d3)
