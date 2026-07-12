print('=== Exception hierarchy tests ===')

try:
    raise KeyError('test')
except LookupError:
    print('1. LookupError caught KeyError: OK')

try:
    raise FileNotFoundError('test')
except OSError:
    print('2. OSError caught FileNotFoundError: OK')

try:
    raise ValueError('bad')
except Exception:
    print('3. Exception caught ValueError: OK')

try:
    raise IndexError('out')
except LookupError:
    print('4. LookupError caught IndexError: OK')

try:
    raise ConnectionRefusedError('refused')
except OSError:
    print('5. OSError caught ConnectionRefusedError: OK')

try:
    raise OverflowError('overflow')
except ArithmeticError:
    print('6. ArithmeticError caught OverflowError: OK')

try:
    raise NotImplementedError('nope')
except RuntimeError:
    print('7. RuntimeError caught NotImplementedError: OK')

try:
    raise ModuleNotFoundError('missing')
except ImportError:
    print('8. ImportError caught ModuleNotFoundError: OK')

print()
print('=== ALL HIERARCHY TESTS PASSED ===')
