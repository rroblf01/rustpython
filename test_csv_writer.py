import csv
print('reader:', csv.reader('a,b'))
result = csv.writer([['a','b'],['1','2']])
print('writer result:', repr(result))
