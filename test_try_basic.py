try:
    raise Exception('test')
except Exception:
    print('caught basic exception: OK')
