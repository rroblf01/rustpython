# Test all new features from subagents v3
print('=== hashlib ===')
import hashlib
h = hashlib.sha256(b'test')
print('sha256:', h[:16])

print()
print('=== base64 ===')
import base64
e = base64.b64encode(b'hello')
print('b64encode:', e)
d = base64.b64decode(e)
print('b64decode:', d)

print()
print('=== uuid ===')
import uuid
u = uuid.uuid4()
print('uuid4:', u)

print()
print('=== string ===')
import string
print('ascii_letters:', string.ascii_letters[:10])

print()
print('=== hex/oct/bin ===')
print('hex(255):', hex(255))
print('oct(255):', oct(255)) 
print('bin(255):', bin(255))

print()
print('=== ascii ===')
print('ascii("hello"):', ascii("hello"))

print()
print('=== memoryview ===')
print('memoryview:', memoryview(b'test'))

print()
print('=== bytearray ===')
ba = bytearray(b'hello')
print('bytearray str:', str(ba))
ba.append(33)
print('after append:', ba)
ba.remove(101)
print('after remove:', ba)

print()
print('=== str methods ===')
s = "hello world"
print('partition:', s.partition(' '))
print('rpartition:', s.rpartition(' '))
print('splitlines:', "a\nb\nc".splitlines())
print('expandtabs:', "a\tb".expandtabs(4))

print()
print('=== list methods ===')
l = [1, 2, 3, 2, 4]
l.remove(2)
print('after remove:', l)
p = l.pop(1)
print('pop(1):', p, 'remaining:', l)

print()
print('=== exception hierarchy ===')
try:
    raise KeyError('test')
except LookupError:
    print('LookupError caught KeyError: OK')
try:
    raise FileNotFoundError('test')
except OSError:
    print('OSError caught FileNotFoundError: OK')

print()
print('=== ALL TESTS PASSED ===')
