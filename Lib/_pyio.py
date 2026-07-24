"""Pure-Python io implementation placeholder.

Real CPython's _pyio.py is a full pure-Python reimplementation of the `io`
module, used as a fallback and as a cross-check against the C-accelerated
`io` module in tests. This interpreter has no separate accelerated/pure-Python
split — `io` is already the only implementation — so re-export it directly
rather than duplicating hundreds of lines of stream-handling logic that
would just have to track `io.py` forever.
"""

from io import *  # noqa: F401,F403
from io import (
    open, IOBase, RawIOBase, BufferedIOBase, TextIOBase,
    BytesIO, StringIO, FileIO,
    BufferedReader, BufferedWriter, BufferedRandom, BufferedRWPair,
    TextIOWrapper, DEFAULT_BUFFER_SIZE, UnsupportedOperation, BlockingIOError,
)
