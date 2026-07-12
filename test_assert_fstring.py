x = 42
s1 = f"{x!r}"
print('s1:', s1)
assert s1 == '42', f"bad: {s1!r}"

s2 = f"{x!s}"
print('s2:', s2)
assert s2 == '42'

s3 = f"{x:>10}"
print('s3:', repr(s3))
print('len:', len(s3))
assert len(s3) == 10, f"expected 10 got {len(s3)}"
print('OK')
