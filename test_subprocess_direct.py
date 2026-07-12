import subprocess
r = subprocess.run(['/bin/echo', 'hello'])
print('run OK:', r)
