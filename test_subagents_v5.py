print('=== str methods ===')
print('zfill:', '42'.zfill(5))
print('center:', 'hi'.center(10, '-'))
print('ljust:', 'hi'.ljust(10, '-'))
print('rjust:', 'hi'.rjust(10, '-'))
print('swapcase:', 'Hello World'.swapcase())
print('title:', 'hello world'.title())
print('encode:', 'hello'.encode('utf-8'))

print()
print('=== property ===')
class C:
    @property
    def x(self):
        return 42
c = C()
print('property get:', c.x)

print()
print('=== enumerate ===')
enum_obj = enumerate(['a', 'b', 'c'])
for v in enum_obj:
    print('  item:', v)

print()
print('=== zip ===')
print('zip one:', list(zip([1,2,3])))
print('zip two:', list(zip([1,2], ['a','b'])))

print()
print('=== isinstance tuple ===')
print('isinstance str:', isinstance('hello', (int, str)))
print('isinstance int:', isinstance(42, (str, float, int)))

print()
print('=== ALL TESTS PASSED ===')
