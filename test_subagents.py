# Test exception hierarchy
print('=== Exception hierarchy ===')
try:
    raise TypeError("test error")
except Exception as e:
    print('except Exception caught TypeError:', str(e)[:30])

try:
    raise ValueError("bad value")
except Exception as e:
    print('except Exception caught ValueError:', str(e)[:25])

# Test finally
print()
print('=== finally ===')
try:
    print('  in try')
finally:
    print('  finally executed: OK')

# Test try/except/else
print()
print('=== try/except/else ===')
try:
    pass
except:
    print('FAIL')
else:
    print('else executed: OK')

# Test __qualname__
print()
print('=== __qualname__ ===')
def f():
    pass
print('function __qualname__:', f.__qualname__)

class MyClass:
    pass
print('class __qualname__:', MyClass.__qualname__)

# Test breakpoint
print()
print('=== breakpoint ===')
breakpoint()
print('breakpoint: OK')

# Test del
print()
print('=== del ===')
x = 42
del x
try:
    print(x)
    print('FAIL')
except NameError:
    print('del x: OK')

# Combined test
print()
print('=== Combined ===')
try:
    raise TypeError("combined test")
except Exception as e:
    print('caught:', str(e)[:25])
finally:
    print('finally after except: OK')

print()
print('=== ALL SUBAGENT TESTS PASSED ===')
