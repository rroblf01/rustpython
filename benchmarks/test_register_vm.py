#!/usr/bin/env python3
"""Test register-based bytecode execution paths.

The register-based instructions (REG_MOV, REG_LOAD_CONST, etc.) are used
when the VM detects hot code paths.  This test verifies that the resulting
register-based execution produces correct results.
"""

def test_reg_mov():
    """Test register move."""
    x = 42
    assert x == 42

def test_reg_arithmetic():
    """Test register-based arithmetic operations."""
    a = 10
    b = 20
    c = a + b
    d = a * b
    e = b - a
    assert c == 30
    assert d == 200
    assert e == 10

def test_reg_local_vars():
    """Test register-based local variable access."""
    x = 1
    y = 2
    z = 3
    result = x + y + z
    assert result == 6

def test_reg_loop():
    """Test register-based loop execution."""
    total = 0
    for i in range(100):
        total += i
    assert total == 4950

def test_reg_nested_loops():
    """Test nested loops with register allocation."""
    result = 0
    for i in range(10):
        for j in range(10):
            result += i * j
    assert result == 2025

def test_reg_list_build():
    """Test register-based list construction."""
    lst = [1, 2, 3, 4, 5]
    assert len(lst) == 5
    assert lst[0] == 1
    assert lst[4] == 5

def test_reg_global_lookup():
    """Test register-based global variable access."""
    # Access builtins and globals repeatedly
    for i in range(100):
        x = len
        y = str
        z = int
    assert callable(x)
    assert callable(y)
    assert callable(z)

def test_reg_function_call():
    """Test register-based function calling."""
    def double(x):
        return x * 2

    for i in range(50):
        result = double(i)
        assert result == i * 2

def test_reg_dict_access():
    """Test register-based dict operations."""
    d = {"a": 1, "b": 2, "c": 3}
    for key in ("a", "b", "c"):
        val = d[key]
        assert val > 0
    d["d"] = 4
    assert len(d) == 4

# Run all tests
if __name__ == "__main__":
    test_reg_mov()
    print("  REG_MOV: OK")
    test_reg_arithmetic()
    print("  REG_arithmetic: OK")
    test_reg_local_vars()
    print("  REG_local_vars: OK")
    test_reg_loop()
    print("  REG_loop: OK")
    test_reg_nested_loops()
    print("  REG_nested_loops: OK")
    test_reg_list_build()
    print("  REG_list_build: OK")
    test_reg_global_lookup()
    print("  REG_global_lookup: OK")
    test_reg_function_call()
    print("  REG_function_call: OK")
    test_reg_dict_access()
    print("  REG_dict_access: OK")
    print("ALL REGISTER TESTS PASSED!")
