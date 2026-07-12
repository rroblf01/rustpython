print('repr TypeError:', repr(TypeError('bad')))
print('print sep:', 'hi', 'there', sep='-')

s = "hello"
print('str format:', f"{s:>10}")

b = b'test'
print('bytes hex:', b.hex())

import _thread; print('_thread OK')
import signal; print('signal OK')
import gc; print('gc OK')
import sysconfig; print('sysconfig OK')
import linecache; print('linecache OK')

print('ALL OK')
