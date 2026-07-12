# Test ALL exception types
tests = ['TypeError', 'ValueError', 'LookupError', 'ArithmeticError', 
         'KeyError', 'IndexError', 'OSError', 'IOError', 
         'FileNotFoundError', 'NotImplementedError', 'RecursionError',
         'ModuleNotFoundError', 'EOFError',
         'ConnectionError', 'TimeoutError']

for t in tests:
    try:
        exec('x = ' + t)
        print(t + ': OK')
    except Exception as e:
        print(t + ': ERROR -', str(e)[:60])
