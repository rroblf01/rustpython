import sys
# Test the exact path construction used by import_module_from_file
for base in sys.path:
    if base.endswith('/'):
        py_path = base + 'weakref.py'
    else:
        py_path = base + '/weakref.py'
    try:
        data = open(py_path).read()
        print('FOUND:', py_path, 'len=', len(data))
        # Try to parse it by using compile
        code = compile(data, py_path, 'exec')
        print('  compiled OK, len=', len(code.co_code) if hasattr(code, 'co_code') else '?')
        exec(code)
        print('  exec OK!')
    except Exception as e:
        print('NOT at', py_path, ':', e)
