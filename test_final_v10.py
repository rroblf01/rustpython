import cmath; print('cmath:', cmath.sqrt(4))
import gzip; print('gzip:', gzip.compress(b'test'))
l = [1,2]; print('list add:', l + [3,4])
l2 = [1]; l2 += [2,3]; print('list iadd:', l2)
print('float int:', int(3.14))
print('hasattr:', hasattr({}, 'keys'))
class C: pass
c = C(); c.x = 1
print('vars:', vars(c))
print('ALL OK')
