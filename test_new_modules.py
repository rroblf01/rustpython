# Test csv module
print("=== CSV MODULE ===")
import csv
data = "a,b,c\n1,2,3\n4,5,6"
result = csv.reader(data)
print("csv.reader:", result)
assert result == [["a", "b", "c"], ["1", "2", "3"], ["4", "5", "6"]], f"Expected 3 rows, got {result}"

back = csv.writer(result)
print("csv.writer:", repr(back))
assert back == "a,b,c\n1,2,3\n4,5,6", f"Expected CSV, got {repr(back)}"

# Test json module
print()
print("=== JSON MODULE ===")
import json
obj = {"b": 2, "a": 1, "c": [3, 4, 5]}
s = json.dumps(obj)
print("json.dumps flat:", s)

s2 = json.dumps(obj, 2)
print("json.dumps indent=2:")
print(s2)

s3 = json.dumps(obj, None, True)
print("json.dumps sort_keys=True:", s3)

loaded = json.loads('{"hello": "world", "nums": [1, 2, 3]}')
print("json.loads:", loaded)

# Test io module
print()
print("=== IO MODULE ===")
import io
s = io.StringIO("hello world")
print("StringIO created:", s)
# Method calls: receive self automatically via BuiltinMethod binding
val = s.getvalue()
print("getvalue():", val)

s2 = io.StringIO("line1\nline2\nline3")
print("readline():", repr(s2.readline()))
print("readline():", repr(s2.readline()))
print("readline():", repr(s2.readline()))
print("readline() at end:", repr(s2.readline()))

s3 = io.StringIO("hello")
print("Initial tell():", s3.tell())
s3.write(" world")
print("After write, getvalue():", s3.getvalue())
print("After write, tell():", s3.tell())
s3.seek(0)
print("After seek(0), tell():", s3.tell())
print("After seek(0), read():", s3.read())

s4 = io.StringIO()
s4.write("test")
print("Empty init + write:", s4.getvalue())

# Test statistics module
print()
print("=== STATISTICS MODULE ===")
import statistics
data = [1, 2, 3, 4, 5]
print("mean:", statistics.mean(data))
print("median:", statistics.median(data))
print("stdev:", statistics.stdev(data))
print("mode (single):", statistics.mode([1, 1, 2, 3, 3, 3]))
print("mode (multi):", statistics.mode([1, 1, 2, 2, 3]))

print()
print("=== ALL TESTS PASSED ===")
