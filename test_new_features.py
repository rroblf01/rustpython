# Test 1: Type annotations (AnnAssign)
x: int = 5
print('Type annotations: x =', x)

# Test 2: Function with type annotations
def f(x: int, y: str) -> bool:
    return True

print('Function annotations: f(1, "a") =', f(1, "a"))

# Test 3: Ellipsis literal
e = ...
print('Ellipsis:', repr(e))

# Test 4: f-string debug support
val = 42
result = f'{val=}'
print('f-string debug:', result)
print('Expected: val=42')

# Test 5: yield from
def gen():
    yield from [1, 2, 3]

print('yield from:', list(gen()))

print('All basic features OK!')
