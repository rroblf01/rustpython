import re
print("re type:", type(re))
print("re dir:", [x for x in dir(re) if 'find' in x.lower()])
print("findall type:", type(re.findall))
print("findall value:", repr(re.findall))
result = re.findall(r"\w+", "a b c")
print("result:", result)
