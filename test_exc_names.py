# Test which exception types work
tests = ['TypeError', 'ValueError', 'LookupError', 'ArithmeticError', 
         'KeyError', 'IndexError', 'OSError', 'IOError', 
         'FileNotFoundError', 'NotImplementedError', 'RecursionError',
         'KeyboardInterrupt', 'GeneratorExit', 'SystemExit',
         'ModuleNotFoundError', 'StopAsyncIteration', 'EOFError',
         'ConnectionError', 'TimeoutError', 'UnicodeDecodeError']

for t in tests:
    try:
        print(t + ':', eval(t))
    except Exception as e:
        print(t + ': ERROR -', str(e)[:50])
