"""Alias for the pragmatic `ast` module's node classes.

Real CPython's `_ast` is the native extension backing `ast.py` (grammar-
generated node types + compile-flag constants). This interpreter's `ast.py`
is already self-contained (doesn't need a native `_ast` at all) — this
module exists only so code that imports `_ast` DIRECTLY (bypassing `ast.py`)
doesn't raise `ImportError`. `PyCF_ONLY_AST`/`PyCF_TYPE_COMMENTS` are real
CPython `compile()` flag values; passing them to this interpreter's own
`compile()` has no special effect (no parser-to-AST bridge exists here —
see `ast.py`'s own doc comment), but the constants themselves are harmless
to expose.
"""

from ast import *  # noqa: F401,F403
from ast import AST, NodeVisitor, NodeTransformer, literal_eval  # noqa: F401

PyCF_ONLY_AST = 1024
PyCF_TYPE_COMMENTS = 4096
PyCF_ALLOW_TOP_LEVEL_AWAIT = 8192
