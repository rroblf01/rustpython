try:
    raise TypeError('bad')
except Exception:
    print('TypeError caught by Exception: OK')

try:
    raise KeyError('missing')
except LookupError:
    print('KeyError caught by LookupError: OK')
