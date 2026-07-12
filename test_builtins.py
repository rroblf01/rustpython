# Test all 5 improved builtins

print('=== 1. delattr ===')
class Foo:
    pass
f = Foo()
f.x = 42
assert hasattr(f, 'x')
delattr(f, 'x')
assert not hasattr(f, 'x')
print('delattr on instance: OK')

# delattr on class
class Bar:
    z = 10
assert hasattr(Bar, 'z')
delattr(Bar, 'z')
assert not hasattr(Bar, 'z')
print('delattr on class: OK')

print()
print('=== 2. memoryview ===')
mv = memoryview(b'hello world')
print('repr:', repr(mv))
b = bytes(mv)
print('bytes:', b)
assert b == b'hello world'
print('memoryview bytes conversion: OK')

# memoryview from string
mv2 = memoryview('hello')
b2 = bytes(mv2)
assert b2 == b'hello'
print('memoryview from str: OK')

print()
print('=== 3. staticmethod ===')
class MyClass:
    @staticmethod
    def static_method(x):
        return x * 2

# Call via class
result = MyClass.static_method(5)
assert result == 10
print('staticmethod via class:', result)

# Call via instance
obj = MyClass()
result2 = obj.static_method(5)
assert result2 == 10
print('staticmethod via instance:', result2)

print()
print('=== 4. classmethod ===')
class MyClass2:
    @classmethod
    def class_method(cls):
        return cls

# Call via class
result = MyClass2.class_method()
assert result is MyClass2
print('classmethod via class:', result)

# Call via instance
obj2 = MyClass2()
result2 = obj2.class_method()
print('classmethod via instance:', result2)

print()
print('=== 5. super() ===')
# Test super() with 2 args
class Base:
    def method(self):
        return 'base'

class Derived(Base):
    def method(self):
        return 'derived'

d = Derived()
s = super(Derived, d)
parent_method = s.method
print('super().method:', parent_method)

# Test super() with no args (returns a super object)
s2 = super()
print('bare super():', repr(s2))

# Test super() with 1 arg
s3 = super(Derived)
print('super(Class):', repr(s3))

print()
print('=== ALL TESTS PASSED ===')
