import sys
print("sys.path:", sys.path)
with open("/tmp/mymod.py", "w") as f:
    f.write("x = 42\n")
    print("Wrote to /tmp/mymod.py")
sys.path.insert(0, "/tmp")
import mymod
print("x:", mymod.x)

import os
os.remove("/tmp/mymod.py")
print("OK")
