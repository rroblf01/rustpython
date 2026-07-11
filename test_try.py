# Test try/except with raise...from
print("start")
try:
    raise ValueError("inner") from RuntimeError("cause")
except ValueError as e:
    print("caught:", e)
    print("cause:", e.__cause__)
print("after except")
print("DONE")
