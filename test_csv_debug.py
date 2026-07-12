import csv
# Single line
print('single:', csv.reader('a,b,c'))
# Multi line
print('multi:', csv.reader('a,b\n1,2'))
# With assert
data = csv.reader('a,b,c')
assert data == [['a', 'b', 'c']], f"got {data}"
print('assert works')
