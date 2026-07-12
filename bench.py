import time
import sys

def bench(name, code, n=1000):
    # Warm up
    for i in range(10): exec(code)
    start = time.time()
    for j in range(n): exec(code)
    elapsed = time.time() - start
    print(f'{name:30s} {elapsed*1000/n:.3f}ms  (x{n})')

print(f'RustPython {sys.version}')
print()

bench('int add', 'x = 1 + 2')
bench('int mul', 'x = 123 * 456')
bench('float add', 'x = 1.5 + 2.5')
bench('str concat', 'x = "hello" + " world"')
bench('str len', 'x = len("hello world")')
bench('list append', 'x = []; x.append(1); x.append(2)')
bench('list index', 'x = [1,2,3]; x.index(2)')
bench('dict get', 'x = {"a":1}; x.get("a")')
bench('dict update', 'd = {"a":1}; d.update({"b":2})')
bench('f-string', 'x = 42; f"value={x}"')
bench('function call', 'def f(x): return x+1; f(5)')
bench('for loop', 's=0; for i in range(100): s+=i')
bench('while loop', 'i=0; s=0; while i<100: s+=i; i+=1')
bench('if-else', 'x=42; if x>10: y=1 else: y=2')
bench('try-except', 'try: x=1; except: x=2')
bench('class create', 'class A: pass; a = A()')
bench('method call', 'class A: def m(self): return 42; a = A(); a.m()')
bench('list comp', '[x*2 for x in range(100)]')
bench('sort', 'x = [3,1,4,1,5,9,2,6]; x.sort()')
bench('import json', 'import json; json.dumps({"a":1})')
