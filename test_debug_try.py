# Debug: print bytecode for match-only function body
# This test verifies what happens with match in function
print('A')
def f():
    match 1:
        case _:
            return 'ok'

# Don't call f yet, just check def succeeded
print('B')

# Now try calling f
try:
    result = f()
    print('C result:', result)
except Exception as e:
    print('Error:', e)
print('DONE')
