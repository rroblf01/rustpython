# Single except clause with LookupError
try:
    raise KeyError('test')
except LookupError:
    print('OK: caught by LookupError')
