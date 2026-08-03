"""Minimal `encodings` package for RustPython.

Real CPython's `encodings` package wires the built-in codecs into the
`codecs` registry. This interpreter's codecs are native (registered
directly), so the package itself only needs to import cleanly — real
code does `import encodings` (or relies on `encodings.aliases`, ...)
without necessarily using its search functions.
"""

import codecs

__all__ = ['aliases', 'search_function']


def search_function(encoding):
    return codecs.lookup(encoding)


class _AliasDict(dict):
    pass


aliases = _AliasDict()
for _name in ('utf-8', 'utf8', 'latin-1', 'latin1', 'iso8859-1', 'ascii', 'utf-16', 'utf-16le', 'utf-16be', 'utf-32', 'utf-32le', 'utf-32be'):
    aliases[_name] = _name


def _incremental_decoder(encoding, errors='strict'):
    return codecs.getincrementaldecoder(encoding)(errors)


def _incremental_encoder(encoding, errors='strict'):
    return codecs.getincrementalencoder(encoding)(errors)
