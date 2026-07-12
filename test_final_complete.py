import weakref
import copy
import types
import struct

# Punto 1: weakref + copy
print('[P1] weakref:', type(weakref.ref(42)).__name__)
x = [1, 2, [3, 4]]
y = copy.deepcopy(x)
x[2][0] = 99
assert y[2][0] == 3
print('[P1] copy.deepcopy: OK')

# Punto 1: struct
print('[P1] struct.calcsize:', struct.calcsize('ii'))
print('[P1] struct.pack/unpack:', struct.unpack('bb', struct.pack('bb', 1, 2)))

# Punto 1: types
print('[P1] types.FunctionType:', type(lambda: None).__name__)

# Punto 2: f-string format specs
x_val = 42
print('[P2] !r:', f"{x_val!r}")
print('[P2] !s:', f"{x_val!s}")
print('[P2] spec:', repr(f"{x_val:>10}"))
print('[P2] combined:', repr(f"{x_val!r:10}"))

# Punto 3: JIT inline cache (test attribute access)
class Obj:
    def __init__(self):
        self.x = 10
o = Obj()
assert o.x == 10

print()
print('=== 3 PUNTOS COMPLETADOS ===')
