import sys
# Test direct read
data = open('/usr/lib/python3.13/weakref.py').read()
print('can read file:', len(data))
# Now try to compile it
import builtins
print('builtins have compile:', hasattr(builtins, 'compile'))
