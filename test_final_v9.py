def f():
    match 1:
        case 1: return 'one'
        case _: return 'other'
print('match first stmt:', f())

import subprocess
r = subprocess.run('echo hello', shell=True)
print('subprocess:', r)

import pickle
print('pickle:', pickle.dumps(42))

import logging
log = logging.getLogger('test')
log.info('test message')

import timeit
t = timeit.timeit('1+1', number=100)
print('timeit:', t)

d = {'a': 1, 'b': 2}
k, v = d.popitem()
print('popitem:', (k, v))

l = [1, 2, 3]
l.clear()
print('cleared list:', l)

print('42 int bool:', (42).__bool__())
print('0 int bool:', (0).__bool__())
print('bit_length 42:', (42).bit_length())

print('ALL OK')
