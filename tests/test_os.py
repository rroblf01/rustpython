import os
print("abspath '.':", os.path.abspath("."))
print("abspath '..':", os.path.abspath(".."))
print("abspath '/':", os.path.abspath("/"))
print("abspath 'nonexistent':", os.path.abspath("nonexistent"))
print("abspath '/foo/bar':", os.path.abspath("/foo/bar"))
print("abspath '':", os.path.abspath(""))
print("getcwd:", os.getcwd())
print("OK")
