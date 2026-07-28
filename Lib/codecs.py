"""Python codecs module - wraps _codecs built-in module."""

import _codecs

def lookup(encoding):
    return _codecs.lookup(encoding)

# `_codecs.lookup()` returns a plain positional tuple (encode, decode,
# stream_reader, stream_writer, name) rather than a real `CodecInfo`
# object with named-attribute access (`.encode`, `.decode`, ...) — index
# into it by position instead. Previously these four all did
# `lookup(encoding).encode`/`.decode`/etc., which raised `AttributeError:
# 'tuple' object has no attribute 'encode'` on every call, breaking
# `codecs.encode()`/`codecs.decode()` (both go through these) entirely.
def getencoder(encoding):
    return lookup(encoding)[0]

def getdecoder(encoding):
    return lookup(encoding)[1]

def getreader(encoding):
    return lookup(encoding)[2]

def getwriter(encoding):
    return lookup(encoding)[3]

def encode(obj, encoding='utf-8', errors='strict'):
    # The codec-level encode/decode functions return `(result, length)`
    # (matching the real C-level codec protocol) — the public
    # `codecs.encode`/`decode` wrappers return just `result`. Previously
    # returned the raw 2-tuple unchanged.
    encoder = getencoder(encoding)
    return encoder(obj, errors)[0]

def decode(obj, encoding='utf-8', errors='strict'):
    decoder = getdecoder(encoding)
    return decoder(obj, errors)[0]

# Standard codec encodings
BOM = b'\xff\xfe'
BOM_BE = b'\xfe\xff'
BOM_LE = b'\xff\xfe'
BOM_UTF8 = b'\xef\xbb\xbf'
BOM_UTF16_LE = b'\xff\xfe'
BOM_UTF16_BE = b'\xfe\xff'

# Error handlers registry
_error_handlers = {}

def register_error(name, handler):
    _error_handlers[name] = handler

def lookup_error(name):
    return _error_handlers.get(name)

def strict_errors(exception):
    raise exception

def replace_errors(exception):
    return ('?', exception.end)

def ignore_errors(exception):
    return ('', exception.end)

register_error('strict', strict_errors)
register_error('replace', replace_errors)
register_error('ignore', ignore_errors)

class IncrementalEncoder:
    def __init__(self, errors='strict'):
        self.errors = errors
    
    def encode(self, input, final=False):
        raise NotImplementedError

class IncrementalDecoder:
    def __init__(self, errors='strict'):
        self.errors = errors
    
    def decode(self, input, final=False):
        raise NotImplementedError

class StreamWriter:
    def __init__(self, stream, errors='strict'):
        self.stream = stream
        self.errors = errors
    
    def write(self, object):
        data, _ = self.stream.write(object)
        return data

class StreamReader:
    def __init__(self, stream, errors='strict'):
        self.stream = stream
        self.errors = errors
    
    def read(self, size=-1):
        return self.stream.read(size)

# Real `codecs.register`/`unregister`: a search function is appended to
# (or removed from) `_codecs`'s own search-function list, consulted by
# `_codecs.lookup()` for encoding names it doesn't recognize directly.
# (A previous version of `register` here did something unrelated —
# `_encodings[encoding.name] = encoding`, registering a *codec object* by
# name into a local dict that `lookup()` never actually consulted — dead
# code that happened to share the name real `codecs.register` uses. Real
# CPython code doing `codecs.register(search_function); ...;
# codecs.unregister(search_function)` — e.g. `test_str.py`'s own test
# setup — needs the real, function-list-based semantics.)
def register(search_function):
    _codecs.register(search_function)

def unregister(search_function):
    _codecs.unregister(search_function)
