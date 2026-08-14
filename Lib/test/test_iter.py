class BasicIterClass:
    def __init__(self, n):
        self.n = n
        self.i = 0

    def __next__(self):
        res = self.i
        if res >= self.n:
            raise StopIteration
        self.i = res + 1
        return res

    def __iter__(self):
        return self


class IteratingSequenceClass:
    def __init__(self, n):
        self.n = n

    def __iter__(self):
        return BasicIterClass(self.n)
