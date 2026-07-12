# Test f-string format specs
x = 42
print('f-string !r:', f"{x!r}")
print('f-string !s:', f"{x!s}")
print('f-string format spec:', repr(f"{x:>10}"))
print('f-string !r:10:', repr(f"{x!r:10}"))
print("=== ALL DONE ===")
