"""Python codecs module - wraps _codecs built-in module."""

import _codecs

def lookup(encoding):
    return _codecs.lookup(encoding)

def getencoder(encoding):
    return lookup(encoding).encode

def getdecoder(encoding):
    return lookup(encoding).decode

def getreader(encoding):
    return lookup(encoding).streamreader

def getwriter(encoding):
    return lookup(encoding).streamwriter

def encode(obj, encoding='utf-8', errors='strict'):
    encoder = getencoder(encoding)
    return encoder(obj, errors)

def decode(obj, encoding='utf-8', errors='strict'):
    decoder = getdecoder(encoding)
    return decoder(obj, errors)

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

# Register encodings
_encodings = {}
def register(encoding):
    _encodings[encoding.name] = encoding

def search_function(encoding):
    return _encodings.get(encoding)

# UTF-8
class utf_8_codec:
    name = 'utf-8'
    @staticmethod
    def encode(input, errors='strict'):
        if isinstance(input, str):
            return (input.encode('utf-8'), len(input))
        raise TypeError('expected str')
    
    @staticmethod
    def decode(input, errors='strict'):
        if isinstance(input, bytes):
            return (input.decode('utf-8'), len(input))
        raise TypeError('expected bytes')

register(utf_8_codec)
