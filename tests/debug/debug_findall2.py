import re
# compile with raw string
c = re.compile(r"\d+")
# findall
result = c.findall("12 abc 34 def 56")
assert result == ["12", "34", "56"], f"re.findall failed: {result}"
result = c.findall("abc")
assert result == [], f"re.findall on non-matching failed: {result}"
# sub
result = c.sub("X", "12 abc 34")
assert result == "X abc X", f"re.sub failed: {result}"
result = c.sub("Y", "abc")
assert result == "abc", f"re.sub on non-matching failed: {result}"
# split
result = c.split("12abc34def56")
assert "abc" in result, f"re.split missing 'abc': {result}"
assert "def" in result, f"re.split missing 'def': {result}"
# pattern and flags
assert c.pattern == r"\d+", f"re.pattern failed: {c.pattern}"
assert c.flags == 0, f"re.flags failed: {c.flags}"
# re module functions
m2 = re.match(r"hello", "hello world")
assert m2 is not None, "re.match function failed"
result = re.findall(r"\w+", "a b c")
assert result == ["a", "b", "c"], f"re.findall function failed: {result}"
print("OK")
