print('no match test')
import subprocess
r = subprocess.run('echo hello', shell=True)
print('subprocess OK')
import pickle
print('pickle:', pickle.dumps(42))
print('DONE')
