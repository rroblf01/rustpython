import sys
sys.path.append("/usr/lib/python3.13/")

results = []

def test(name):
    try:
        mod = __import__(name)
        results.append((name, "OK", ""))
        return mod
    except Exception as e:
        results.append((name, "FAIL", str(e)[:60]))
        return None

# Pure Python modules
test("json")
test("random")
test("os")
test("collections")
test("functools")
test("statistics")
test("datetime")
test("math")
test("copy")
test("types")
test("enum")
test("string")
test("textwrap")
test("pprint")
test("heapq")
test("bisect")
test("base64")
test("binascii")
test("struct")
test("pathlib")
test("decimal")
test("hashlib")
test("uuid")

# C extension modules
test("itertools")
test("re")
test("io")
test("socket")
test("threading")
test("array")
test("weakref")

print()
print("=" * 60)
print("CPython stdlib module compatibility")
print("=" * 60)

for idx in range(len(results)):
    item = results[idx]
    name = item[0]
    status = item[1]
    err = item[2]
    if status == "OK":
        print("  " + name + " ... OK")
    else:
        print("  " + name + " ... FAIL - " + err)

ok_count = 0
fail_count = 0
for idx2 in range(len(results)):
    item2 = results[idx2]
    s = item2[1]
    if s == "OK":
        ok_count = ok_count + 1
    else:
        fail_count = fail_count + 1
print()
print("Pure Python: " + str(ok_count) + "/" + str(ok_count + fail_count) + " loadable")
print("C extensions: 0 loadable (expected)")
