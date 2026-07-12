def f():
    match 1:
        case 1: return 'one'
        case _: return 'other'
print('match first stmt:', f())

import subprocess
r = subprocess.run('echo hello', shell=True)
print('subprocess.run:', r)

import timeit
print('timeit imported OK')
print('ALL OK')
