import subprocess
try:
    r = subprocess.run('echo hello', shell=True)
    print('run OK:', r)
except Exception as e:
    print('run error:', e)
try:
    out = subprocess.check_output('echo hello', shell=True)
    print('check_output OK:', out)
except Exception as e:
    print('check_output error:', e)
print('DONE')
