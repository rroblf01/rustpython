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

# Error handlers registry — shared with _codecs (real CPython keeps this
# in the _codecs C module; this stub delegates to it so register_error/
# lookup_error/unregister_error and _codecs._unregister_error agree).
import _codecs as _codecs

def register_error(name, handler):
    _codecs._register_error(name, handler)

def lookup_error(name):
    return _codecs.lookup_error(name)

def unregister_error(name):
    return _codecs._unregister_error(name)

def strict_errors(exception):
    raise exception

def replace_errors(exception):
    return ('?', exception.end)

def ignore_errors(exception):
    return ('', exception.end)

register_error('strict', strict_errors)
register_error('replace', replace_errors)
register_error('ignore', ignore_errors)

backslashreplace_errors = _codecs.backslashreplace_errors
xmlcharrefreplace_errors = _codecs.xmlcharrefreplace_errors
surrogateescape_errors = _codecs.surrogateescape_errors
surrogatepass_errors = _codecs.surrogatepass_errors
register_error('backslashreplace', backslashreplace_errors)
register_error('xmlcharrefreplace', xmlcharrefreplace_errors)
register_error('surrogateescape', surrogateescape_errors)
register_error('surrogatepass', surrogatepass_errors)

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

# Base class for codec implementations (testcodec.py subclasses it).
class Codec:
    def encode(self, input, errors='strict'):
        raise NotImplementedError

    def decode(self, input, errors='strict'):
        raise NotImplementedError

def make_identity_dict(rng):
    return {i: i for i in rng}

def make_encoding_map(decoding_map):
    m = {}
    for k, v in decoding_map.items():
        if v not in m:
            m[v] = k
        else:
            m[v] = None
    return m

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

charmap_encode = _codecs.charmap_encode
charmap_decode = _codecs.charmap_decode
charmap_build = _codecs.charmap_build
