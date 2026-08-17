"""Dynamic type creation and inspection.

This module extends the native _types_native module with additional functions
that require calling back into Python (new_class, prepare_class, etc.).
"""

import sys as _sys

# Import everything from the native types module
from _types_native import *

def prepare_class(name, bases=(), kwds=None):
    """Create the namespace for a new class."""
    if kwds is None:
        kwds = {}
    
    meta = type
    for base in reversed(bases):
        if hasattr(type(base), '__mro__'):
            bt = type(base)
            if bt is not type:
                meta = bt
                break
    
    if 'metaclass' in kwds:
        meta = kwds.pop('metaclass')
    
    if hasattr(meta, '__prepare__'):
        ns = meta.__prepare__(name, bases, **kwds)
    else:
        ns = {}
    
    return meta, ns, kwds

def new_class(name, bases=(), kwds=None, exec_body=None, **other_kwds):
    """Create a new class dynamically."""
    if kwds is not None and not isinstance(kwds, dict):
        raise TypeError("kwds must be a dict")
    if kwds is None:
        all_kwds = dict(other_kwds)
    else:
        all_kwds = {**kwds, **other_kwds}
    if bases is None:
        bases = ()
    
    meta, ns, prepared_kwds = prepare_class(name, bases, all_kwds)
    
    if exec_body is not None:
        exec_body(ns)
    
    return meta(name, bases, ns, **all_kwds)

# Make all names available
_all_names = [n for n in dir(_sys.modules['_types_native']) if not n.startswith('_')]
for n in _all_names:
    if n not in dir():
        globals()[n] = getattr(_sys.modules['_types_native'], n)

__all__ = [n for n in dir() if not n.startswith('_') and n not in ('_sys',)]
