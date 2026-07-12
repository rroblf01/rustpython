import _weakref
print('_weakref OK')
print('ref:', _weakref.ref)
r = _weakref.ref(42)
print('r():', r())
