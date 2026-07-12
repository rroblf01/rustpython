import subprocess
r = subprocess.run('echo hello', shell=True)
print('run:', r)
