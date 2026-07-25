"""Interface to the Expat non-validating XML parser.

Real CPython's own `xml/parsers/expat/__init__.py` is a thin re-export of
the native `pyexpat` module. This interpreter's `pyexpat` (`Lib/pyexpat.py`)
is itself still a placeholder stub (handler setters that don't store their
callbacks, no real tag/attribute/entity parsing) — `xml.dom.minidom`/
`xml.sax` can therefore IMPORT cleanly through this module, but actual
parsing via `expatbuilder`/`_ExpatParser` will not produce a real DOM/SAX
event stream yet. A genuine expat-compatible parser is a separate, sizeable
follow-up.
"""
from pyexpat import *  # noqa: F401,F403
from pyexpat import ExpatError, errors  # noqa: F401
