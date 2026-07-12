# Test #1: _ as variable
_ = 42
print("_ as variable:", _)

# Test _ in for loop
total = 0
for _ in range(3):
    total += 1
print("for _ in range:", total)

# Test #4: Multi-line lists with trailing comma
nums = [1, 2, 3,]
print("trailing comma list:", nums)

nums2 = [
    1,
    2,
    3,
]
print("multi-line list:", nums2)

# Test #5: Type annotations
def f(x: int):
    return x

print("type annotation:", f(5))
