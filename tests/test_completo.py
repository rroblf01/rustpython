# Test completo de funcionalidades implementadas

# === 1. f-strings format spec ===
s = "hello"
assert f"{s:>10}" == "     hello"
assert f"{s:<10}" == "hello     "
assert f"{s:^11}" == "   hello   "
assert f"{s!r}" == "'hello'"
assert f"{42!s}" == "42"
assert f"{42!a}" == "42"
assert f"{42:10}" == "        42"
print("1. f-strings: OK")

# === 2. repr exceptions ===
assert repr(TypeError("bad msg")) == "TypeError('bad msg')"
assert repr(ValueError("v")) == "ValueError('v')"
print("2. repr exceptions: OK")

# === 3. print kwargs ===
print("a", "b", sep=":", end="|")
print(" 3. print sep/end: OK")

# === 4. str.__format__ ===
s = "hi"
assert format(s, ">10") == "        hi"
assert format(s, "<10") == "hi        "
assert format(s, "^10") == "    hi    "
assert f"{s:*^10}" == "****hi****"
print("4. str.__format__: OK")

# === 5. bytes.hex() and decode() ===
b = b"test"
assert b.hex() == "74657374"
assert b.decode() == "test"
assert b.decode("utf-8") == "test"
print("5. bytes.hex/decode: OK")

# === 6. Native modules ===
import _thread
import signal
import gc
import sysconfig
import linecache
assert signal.SIGINT == 2
assert signal.SIGTERM == 15
assert gc.collect() == 0
gc.enable()
gc.disable()
assert sysconfig.get_config_var("prefix") is None
assert linecache.getline("nope", 1) == ""
linecache.clearcache()
print("6. Modules: OK")

# === 7. Match first-statement fix ===
def match_only():
    match 1:
        case _:
            return "ok"
    return "nope"

assert match_only() == "ok"

def match_return():
    match 42:
        case 42:
            return "matched 42"
        case _:
            return "default"
    return "not reached"

assert match_return() == "matched 42"
print("7. Match first-stmt: OK")

# === 8. For-else ===
def for_else():
    for x in [1, 2, 3]:
        if x == 5:
            break
    else:
        return "else executed"
    return "break occurred"

assert for_else() == "else executed"

def for_break():
    for x in [1, 2, 3]:
        if x == 2:
            break
    else:
        return "else executed"
    return "break occurred"

assert for_break() == "break occurred"
print("8. For-else: OK")

# === 9. While-else ===
def while_else():
    i = 0
    while i < 3:
        i += 1
        if i == 5:
            break
    else:
        return "else executed"
    return "break occurred"

assert while_else() == "else executed"

def while_break():
    i = 0
    while i < 3:
        i += 1
        if i == 2:
            break
    else:
        return "else executed"
    return "break occurred"

assert while_break() == "break occurred"
print("9. While-else: OK")

# === 10. Augmented assignment ===
x = [1, 2, 3]
x[0] += 10
assert x[0] == 11
print("10. Augmented assignment: OK")

print("\n=== ALL TESTS PASSED ===")
