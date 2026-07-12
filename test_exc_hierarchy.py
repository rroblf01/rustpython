print('=== simple exception test ===')
try:
    raise KeyError('test')
except LookupError:
    print('LookupError caught KeyError: OK')

try:
    raise FileNotFoundError('test')
except OSError:
    print('OSError caught FileNotFoundError: OK')

try:
    raise TypeError('bad type')
except Exception:
    print('Exception caught TypeError: OK')

print()
print('=== ALL DONE ===')
