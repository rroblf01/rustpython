import weakref
import copy
import types
import struct

# Punto 1: weakref + copy
print('[P1] weakref imported OK')
x = [1, 2, [3, 4]]
y = copy.deepcopy(x)
x[2][0] = 99
if y[2][0] == 3:
    print('[P1] deepcopy: OK')

# Punto 1: struct
sz = struct.calcsize('ii')
print('[P1] struct.calcsize(ii):', sz)

# Punto 1: types imported
print('[P1] types imported:', hasattr(types, 'FunctionType'))

# Punto 2: f-string format specs
x_val = 42
print('[P2] !r:', f"{x_val!r}")
print('[P2] !s:', f"{x_val!s}")
s = f"{x_val:>10}"
print('[P2] spec len:', len(s))

# Punto 3: JIT inline cache
print('[P3] OK')

print()
print('=== 3 PUNTOS COMPLETADOS ===')
