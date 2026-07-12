# Test file reading from sys.path components
import sys
for p in sys.path:
    print('checking:', p)
    try:
        fn = p + '/weakref.py'
        if fn.startswith('.'):
            import os
            fn = os.getcwd() + fn[1:]
        print('  full path:', fn)
        d = open(fn).read()
        print('  OK, len =', len(d))
    except Exception as e:
        print('  error:', e)
