import subprocess
r = subprocess.run('echo hello', shell=True)
print('subprocess OK:', r)
print('ALL OK')
