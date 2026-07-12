print('=== platform ===')
import platform
print('platform:', platform.platform())
print('machine:', platform.machine())
print('system:', platform.system())
print('py_impl:', platform.python_implementation())

print()
print('=== getpass ===')
import getpass
print('getuser:', getpass.getuser())

print()
print('=== tempfile ===')
import tempfile
print('tempdir:', tempfile.tempdir)

print()
print('=== shutil ===')
import shutil
print('shutil imported OK')

print()
print('=== ord/chr ===')
print('ord A:', ord('A'))
print('chr 65:', chr(65))

print()
print('=== __delattr__ ===')
class C:
    def __init__(self):
        self.x = 1
        self.y = 2
c = C()
del c.x
print('hasattr x:', hasattr(c, 'x'))
print('hasattr y:', hasattr(c, 'y'))

print()
print('=== __reversed__ ===')
l = [1, 2, 3]
print('reversed list:', list(reversed(l)))
t = (1, 2, 3)
print('reversed tuple:', list(reversed(t)))

print()
print('=== __sizeof__ ===')
print('list sizeof:', [1,2,3].__sizeof__())
print('str sizeof:', 'hello'.__sizeof__())

print()
print('=== ALL TESTS PASSED ===')
