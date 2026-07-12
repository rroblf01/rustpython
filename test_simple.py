# Test __next__ method specifically
class MyIter:
    def __init__(self, n):
        self.n = n
        self.i = 0
    def __next__(self):
        if self.i >= self.n:
            raise StopIteration
        self.i += 1
        return self.i

obj = MyIter(3)
print("n =", obj.n, ", i =", obj.i)

# Call __next__ directly 
result = obj.__next__()
print("After __next__(): i =", obj.i, ", result =", result)
print("OK")
