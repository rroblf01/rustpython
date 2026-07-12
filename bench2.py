import timeit
print('RustPython benchmark')
print()

tests = [
    ('int add', 'x = 1 + 2'),
    ('int mul', 'x = 123 * 456'),
    ('str concat', 'x = "hello" + " world"'),
    ('list append', 'x = []; x.append(1)'),
    ('dict get', 'x = {"a":1}; x.get("a")'),
    ('f-string', 'x = 42; f"value={x}"'),
    ('function call', 'def f(x): return x+1; f(5)'),
    ('for loop', 's=0; for i in range(100): s+=i'),
    ('if-else', 'x=42; if x>10: y=1 else: y=2'),
    ('try-except', 'try: x=1; except: x=2'),
    ('class create', 'class A: pass'),
    ('list comp', '[1, 2, 3]'),
    ('sort', 'x = [3,1,4,1,5,9,2,6]; x.sort()'),
]

for name, code in tests:
    t = timeit.timeit(code, number=10000)
    print(f'{name:20s} {t*1000:.3f}ms')
