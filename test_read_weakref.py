import sys
print('sys.path:', sys.path)
try:
    exec(open('/usr/lib/python3.13/weakref.py').read())
    print('weakref source loaded OK')
except Exception as e:
    print('Error loading weakref:', e)
