# Check if file reading works
s = open('/usr/lib/python3.13/weakref.py').read()
print('read', len(s), 'bytes')
print('first line:', s.split('\n')[0][:50])
