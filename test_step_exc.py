# Step by step - which except clauses work?
try:
    raise KeyError('test')
except Exception:
    print('A. KeyError caught by Exception: OK')

try:
    raise KeyError('test')
except LookupError:
    print('B. KeyError caught by LookupError: OK')
