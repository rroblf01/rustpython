# Test 1: Basic try/except with Exception base class
print("=== Test 1: catch TypeError with except Exception ===")
try:
    raise TypeError("bad type")
except Exception as e:
    print("PASS: Caught TypeError with except Exception")

print()

# Test 2: catch ValueError with except Exception  
print("=== Test 2: catch ValueError with except Exception ===")
try:
    raise ValueError("bad value")
except Exception as e:
    print("PASS: Caught ValueError with except Exception")

print()

# Test 3: catch multiple exception types
print("=== Test 3: catch specific exceptions ===")
try:
    raise TypeError("specific type")
except ValueError:
    print("FAIL: Caught ValueError instead of TypeError")
except TypeError as e:
    print("PASS: Caught specific TypeError")

print()

# Test 4: nested try/except
print("=== Test 4: nested try/except ===")
try:
    try:
        raise TypeError("inner")
    except ValueError:
        print("FAIL: Inner ValueError caught")
except Exception as e:
    print("PASS: Outer Exception caught TypeError from inner")

print()

# Test 5: try with normal completion (no exception)
print("=== Test 5: normal try completion ===")
try:
    x = 42
except:
    print("FAIL: Exception handler reached without exception")
else:
    print("PASS: Normal completion (x =", x, ")")

print()

# Test 6: bare except
print("=== Test 6: bare except ===")
try:
    raise RuntimeError("test")
except:
    print("PASS: Bare except caught RuntimeError")

print()

# Test 7: except with Exception hierarchy
print("=== Test 7: except with Exception hierarchy ===")
try:
    raise KeyError("missing key")
except Exception as e:
    print("PASS: KeyError caught by Exception")

print()

# Test 8: bare raise handler cleanup
print("=== Test 8: stack cleanup after handler ===")
def test_stack_cleanup():
    try:
        raise ValueError("test")
    except ValueError:
        # No 'as e' - exception should be cleaned up by POP_EXCEPT
        pass
    # If stack is clean, we can do normal operations
    x = 1 + 1
    return x
result = test_stack_cleanup()
print("PASS: Stack clean, result =", result)

print()

# Test 9: finally blocks
print("=== Test 9: finally blocks ===")
try:
    try:
        raise TypeError("test")
    finally:
        print("PASS: Inner finally executed")
except Exception as e:
    print("PASS: Outer Exception caught from inner")

print()

# Test 10: catch NameError with except Exception
print("=== Test 10: catch NameError with except Exception ===")
try:
    # Use eval to trigger NameError without depending on Python version
    eval("undefined_var")
except Exception as e:
    print("PASS: NameError caught by except Exception")

print()

print("=== ALL TESTS COMPLETE ===")
