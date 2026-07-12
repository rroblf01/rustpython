import subprocess
r = subprocess.run('/bin/ls /tmp', shell=True)
print('run:', r)
