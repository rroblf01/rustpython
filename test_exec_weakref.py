import sys
# Direct read test
data = open('/usr/lib/python3.13/weakref.py').read()
print('read OK, len =', len(data))
# Try to exec it
exec(data)
print('exec OK!')
