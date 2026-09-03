"""Pickle module with file-based serialize/deserialize."""

from _pickle import *
from _pickle import _loads

__all__ = [
    'PickleError', 'PicklingError', 'UnpicklingError',
    'Pickler', 'Unpickler',
    'dump', 'dumps', 'load', 'loads',
    'encode_long', 'decode_long',
    'HIGHEST_PROTOCOL', 'DEFAULT_PROTOCOL',
    'PickleBuffer', 'bytes_types',
]

# Pickle opcodes (verbatim from CPython 3.14 pickle.py) — needed so
# pickletools.assure_pickle_consistency() finds the same opcodes in
# pickle.py as in its own table. Our Pickler/Unpickler come from the
# native _pickle stub; opcodes are protocol constants only.
MARK           = b'('
STOP           = b'.'
POP            = b'0'
POP_MARK       = b'1'
DUP            = b'2'
FLOAT          = b'F'
INT            = b'I'
BININT         = b'J'
BININT1        = b'K'
LONG           = b'L'
BININT2        = b'M'
NONE           = b'N'
PERSID         = b'P'
BINPERSID      = b'Q'
REDUCE         = b'R'
STRING         = b'S'
BINSTRING      = b'T'
SHORT_BINSTRING= b'U'
UNICODE        = b'V'
BINUNICODE     = b'X'
APPEND         = b'a'
BUILD          = b'b'
GLOBAL         = b'c'
DICT           = b'd'
EMPTY_DICT     = b'}'
APPENDS        = b'e'
GET            = b'g'
BINGET         = b'h'
INST           = b'i'
LONG_BINGET    = b'j'
LIST           = b'l'
EMPTY_LIST     = b']'
OBJ            = b'o'
PUT            = b'p'
BINPUT         = b'q'
LONG_BINPUT    = b'r'
SETITEM        = b's'
TUPLE          = b't'
EMPTY_TUPLE    = b')'
SETITEMS       = b'u'
BINFLOAT       = b'G'
PROTO          = b'\x80'
NEWOBJ         = b'\x81'
EXT1           = b'\x82'
EXT2           = b'\x83'
EXT4           = b'\x84'
TUPLE1         = b'\x85'
TUPLE2         = b'\x86'
TUPLE3         = b'\x87'
NEWTRUE        = b'\x88'
NEWFALSE       = b'\x89'
LONG1          = b'\x8a'
LONG4          = b'\x8b'
BINBYTES       = b'B'
SHORT_BINBYTES = b'C'
SHORT_BINUNICODE = b'\x8c'
BINUNICODE8      = b'\x8d'
BINBYTES8        = b'\x8e'
EMPTY_SET        = b'\x8f'
ADDITEMS         = b'\x90'
FROZENSET        = b'\x91'
NEWOBJ_EX        = b'\x92'
STACK_GLOBAL     = b'\x93'
MEMOIZE          = b'\x94'
FRAME            = b'\x95'
BYTEARRAY8       = b'\x96'
NEXT_BUFFER      = b'\x97'
READONLY_BUFFER  = b'\x98'

__all__.extend([
    'MARK', 'STOP', 'POP', 'POP_MARK', 'DUP', 'FLOAT', 'INT', 'BININT',
    'BININT1', 'LONG', 'BININT2', 'NONE', 'PERSID', 'BINPERSID', 'REDUCE',
    'STRING', 'BINSTRING', 'SHORT_BINSTRING', 'UNICODE', 'BINUNICODE',
    'APPEND', 'BUILD', 'GLOBAL', 'DICT', 'EMPTY_DICT', 'APPENDS', 'GET',
    'BINGET', 'INST', 'LONG_BINGET', 'LIST', 'EMPTY_LIST', 'OBJ', 'PUT',
    'BINPUT', 'LONG_BINPUT', 'SETITEM', 'TUPLE', 'EMPTY_TUPLE', 'SETITEMS',
    'BINFLOAT', 'PROTO', 'NEWOBJ', 'EXT1', 'EXT2', 'EXT4', 'TUPLE1',
    'TUPLE2', 'TUPLE3', 'NEWTRUE', 'NEWFALSE', 'LONG1', 'LONG4',
    'BINBYTES', 'SHORT_BINBYTES', 'SHORT_BINUNICODE', 'BINUNICODE8',
    'BINBYTES8', 'EMPTY_SET', 'ADDITEMS', 'FROZENSET', 'NEWOBJ_EX',
    'STACK_GLOBAL', 'MEMOIZE', 'FRAME', 'BYTEARRAY8', 'NEXT_BUFFER',
    'READONLY_BUFFER',
])


def dump(obj, file, protocol=None, *, fix_imports=True, buffer_callback=None):
    """Serialize object to open file."""
    data = dumps(obj, protocol=protocol, fix_imports=fix_imports)
    file.write(data)


# Private C-accelerated aliases (CPython exposes _dump/_dumps from _pickle;
# test_pickle references them). Same semantics via the public functions.
def _dump(obj, file, protocol=None, *, fix_imports=True, buffer_callback=None):
    return dump(obj, file, protocol=protocol, fix_imports=fix_imports,
                buffer_callback=buffer_callback)


def _dumps(obj, protocol=None, *, fix_imports=True, buffer_callback=None):
    return dumps(obj, protocol=protocol, fix_imports=fix_imports,
                 buffer_callback=buffer_callback)


def _load(file, *, fix_imports=True, encoding='ASCII', errors='strict',
          buffers=None):
    return load(file, fix_imports=fix_imports, encoding=encoding,
                errors=errors, buffers=buffers)


def load(file, *, fix_imports=True, encoding='ASCII', errors='strict', buffers=None):
    """Deserialize object from open file."""
    data = file.read()
    return loads(data, fix_imports=fix_imports, encoding=encoding, errors=errors)
