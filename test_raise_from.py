# Test 2: raise exc from cause
try:
    raise ValueError("original") from ValueError("cause")
except ValueError as e:
    print("caught ValueError")
