import subprocess
r = subprocess.run('/usr/bin/env echo hello', shell=True)
print('run result:', r)
