import sys

results = []

def test(name):
    try:
        mod = __import__(name)
        results.append((name, "OK", ""))
    except Exception as e:
        results.append((name, "FAIL", str(e)[:80]))

test("json")
test("random")
test("os")
test("collections")
test("functools")
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
test("hashlib")
test("uuid")
test("decimal")
test("pathlib")
test("statistics")
test("numbers")
test("abc")

print("=" * 60)
print("CPython stdlib module compatibility")
print("=" * 60)

for idx in range(len(results)):
    item = results[idx]
    name = item[0]
    status = item[1]
    err = item[2]
    padding = " " * (15 - len(name))
    if status == "OK":
        print("  " + name + padding + "[OK]")
    else:
        print("  " + name + padding + "[FAIL] " + err)

ok_count = 0
fail_count = 0
for idx2 in range(len(results)):
    s = results[idx2][1]
    if s == "OK":
        ok_count = ok_count + 1
    else:
        fail_count = fail_count + 1
print()
print(str(ok_count) + " passed, " + str(fail_count) + " failed")
print(str(ok_count) + "/" + str(len(results)) + " modules loadable")
