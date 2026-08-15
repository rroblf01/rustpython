"""Pickle module with file-based serialize/deserialize."""

from _pickle import *
from _pickle import _loads


def dump(obj, file, protocol=None, *, fix_imports=True, buffer_callback=None):
    """Serialize object to open file."""
    data = dumps(obj, protocol=protocol, fix_imports=fix_imports)
    file.write(data)


def load(file, *, fix_imports=True, encoding='ASCII', errors='strict', buffers=None):
    """Deserialize object from open file."""
    data = file.read()
    return loads(data, fix_imports=fix_imports, encoding=encoding, errors=errors)
