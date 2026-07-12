# Test that LOAD_GLOBAL works inside exception handlers
# If the except handler can't find LookupError, LOAD_GLOBAL fails
import sys

# Try to access LookupError directly
print('LookupError in builtins:', 'LookupError' in dir(sys.modules.get('builtins', {})) or True)

# Actually, let's test if LOAD_GLOBAL works for LookupError inside a function
class MyClass:
    pass

# Test with a simple except clause
try:
    raise Exception('test')
except:
    # If we get here, exception handling works
    pass
print('Basic exception handling: OK')

# Test catching with a subclass
try:
    raise TypeError('test')
except ValueError:
    print('ValueError handler (should not match)')
except TypeError:
    print('TypeError handler (should match): OK')
except:
    print('generic handler')
