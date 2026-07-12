# Bug 4: f-string debug expr= format
print('START')

# Test basic
val = 42
result = f'{val=}'
print('result:', result)
print('expected: val=42')

# Test with multiple expressions
a = 1
b = 2
result2 = f'{a=} {b=}'
print('result2:', result2)
print('expected: a=1 b=2')

# Test with string values
name = 'world'
result3 = f'{name=}'
print('result3:', result3)
print('expected: name=world')

print('DONE')
