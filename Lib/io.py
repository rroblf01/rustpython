"""Core tools for working with streams.

Minimal wrapper over RustPython's built-in _io module.
"""

import _io

__all__ = [
    "open", "IOBase", "RawIOBase", "BufferedIOBase", "TextIOBase",
    "BytesIO", "StringIO", "FileIO",
    "BufferedReader", "BufferedWriter", "BufferedRandom", "BufferedRWPair",
    "TextIOWrapper",
    "DEFAULT_BUFFER_SIZE", "UnsupportedOperation", "BlockingIOError",
]

# ── Direct re-exports from _io ──────────────────────────────────────────────

open = _io.open
open_code = _io.open_code

IOBase = _io.IOBase
RawIOBase = _io.RawIOBase
BufferedIOBase = _io.BufferedIOBase
TextIOBase = _io.TextIOBase

BytesIO = _io.BytesIO
StringIO = _io.StringIO
FileIO = _io.FileIO

BufferedReader = _io.BufferedReader
BufferedWriter = _io.BufferedWriter
BufferedRandom = _io.BufferedRandom
BufferedRWPair = _io.BufferedRWPair
TextIOWrapper = _io.TextIOWrapper

DEFAULT_BUFFER_SIZE = _io.DEFAULT_BUFFER_SIZE
UnsupportedOperation = _io.UnsupportedOperation
BlockingIOError = _io.BlockingIOError
IncrementalNewlineDecoder = _io.IncrementalNewlineDecoder
text_encoding = _io.text_encoding
