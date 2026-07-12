import weakref
print('weakref imported OK')
r = weakref.ref(42)
print('ref created OK')
print('ref() =', r())
