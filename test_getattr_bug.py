class Foo:
    def __init__(self):
        self.__dict__["x"] = 42
        self.y = 100

    def __getattr__(self, name):
        print("__getattr__ called with", name)
        return 999

    def __setattr__(self, name, value):
        print("__setattr__ called:", name, value)
        # In __init__, self.__dict__["x"] = 42 sets x
        # Then self.y = 100 calls __setattr__
        # In __setattr__, we try to access self.x 
        # If this works (finds in __dict__), ok
        # If it falls through to __getattr__, we get 999
        try:
            x_val = self.x
            print("  self.x =", x_val)
        except AttributeError:
            print("  self.x raised AttributeError")
        object.__setattr__(self, name, value)

f = Foo()
print("done, f.x =", f.x)
print("done, f.y =", f.y)
