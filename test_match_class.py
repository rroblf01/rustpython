# Test match/case class pattern with keyword args
class Point:
    def __init__(self, x, y):
        self.x = x
        self.y = y

p = Point(0, 1)
match p:
    case Point(x=0, y=1):
        print("matched Point(x=0, y=1)")
    case _:
        print("no match")
