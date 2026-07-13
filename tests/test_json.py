# Test json module
import json

# dumps/loads
data = {"name": "test", "value": 42}
s = json.dumps(data)
assert '"name"' in s
assert '"value"' in s

d = json.loads(s)
assert d["name"] == "test"
assert d["value"] == 42

# List
s2 = json.dumps([1, 2, 3])
assert s2 == "[1, 2, 3]"
d2 = json.loads(s2)
assert d2 == [1, 2, 3]

# None, bool
assert json.dumps(None) == "null"
assert json.dumps(True) == "true"
assert json.dumps(False) == "false"

print("OK")
