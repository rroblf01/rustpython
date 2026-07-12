# Test hierarchy step by step
try:
    raise KeyError('test')
except Exception:
    print('KeyError caught by Exception: OK')

try:
    raise KeyError('test')
except LookupError:
    print('KeyError caught by LookupError: OK')
