import csv
import json
import io
import statistics
print('all imports OK')
print('csv.reader:', csv.reader('a,b'))
print('json.loads:', json.loads('{"a":1}'))
buf = io.StringIO('test')
print('StringIO.read:', buf.read())
print('statistics.mean:', statistics.mean([1,2]))
