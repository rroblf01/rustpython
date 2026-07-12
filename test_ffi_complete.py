# Test: weakref module loads via _weakref native module
try:
    import weakref
    print('[OK] weakref imported')

    class MyObj:
        pass

    o = MyObj()
    r = weakref.ref(o)
    result = r()
    print('[OK] weakref.ref() works:', result is o)
except Exception as e:
    print('[FAIL] weakref:', e)

# Test: copy module works
try:
    import copy
    print('[OK] copy imported')

    x = [1, 2, [3, 4]]
    y = copy.deepcopy(x)
    x[2][0] = 99
    assert y[2][0] == 3, "deepcopy should not share nested lists"
    print('[OK] deepcopy works:', x, '->', y)
except Exception as e:
    print('[FAIL] copy:', e)

# Test: collections.abc (depends on _collections_abc)
try:
    import collections
    print('[OK] collections imported')
    print('[OK] collections.abc:', hasattr(collections, 'abc'))
except Exception as e:
    print('[FAIL] collections:', e)

# Test: operator module
try:
    import operator
    print('[OK] operator imported')
    assert operator.add(1, 2) == 3
    print('[OK] operator.add(1, 2) =', operator.add(1, 2))
except Exception as e:
    print('[FAIL] operator:', e)

# Test: f-string format specs (punto 2)
try:
    x = 42
    s1 = f"{x!r}"
    print('[OK] f-string !r:', s1)
    s2 = f"{x!s}"
    print('[OK] f-string !s:', s2)
    s3 = f"{x:>10}"
    print('[OK] f-string format spec:', repr(s3))
except Exception as e:
    print('[FAIL] f-string formats:', e)

print()
print("=== ALL TESTS DONE ===")
