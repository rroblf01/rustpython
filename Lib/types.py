"""Dynamic type creation and inspection.

This module extends the native _types_native module with additional functions
that require calling back into Python (new_class, prepare_class, etc.).
"""

import sys as _sys

# Import everything from the native types module
from _types_native import *

# The remaining CPython `types` type aliases, computed exactly as
# `Lib/types.py`'s pure-Python fallback block does (it only runs when the C
# `_types` module is missing, which is this interpreter's situation).
# Without them, importing real CPython's `dataclasses`/`traceback`/
# `xmlrpc.server` dies at `AttributeError: module 'types' has no attribute
# 'EllipsisType'`.
EllipsisType = type(Ellipsis)
NotImplementedType = type(NotImplemented)
# CPython computes these as `type(FunctionType.__code__)` /
# `type(FunctionType.__globals__)` — i.e. the types of the C-level getset /
# member descriptor OBJECTS sitting in the function type's dict. This VM
# exposes `__code__`/`__globals__`/... as native pseudo-attributes instead,
# so there is no descriptor object to take a type from (and the function
# type's own `__dict__` is empty). Define the two names as distinct types
# with CPython's exact type names instead of leaving them undefined: that
# keeps every `isinstance(..., types.MemberDescriptorType)` /
# `isinstance(..., types.GetSetDescriptorType)` check (dataclasses,
# inspect, rlcompleter) working structurally — False for a non-descriptor,
# which is the answer they want — instead of an `AttributeError` at import
# time, which is what took down the real `dataclasses`/`inspect` sources.
GetSetDescriptorType = type('getset_descriptor', (), {})
MemberDescriptorType = type('member_descriptor', (), {})
# CPython gets `FrameType` from a live traceback
# (`type(exc.__traceback__.tb_frame)`). Neither route works at OUR import
# time: `types` is imported transitively (`contextvars` -> `abc` ->
# `_weakrefset`) while the VM is still bootstrapping, and this VM does not
# yet attach a real `__traceback__` to caught exceptions, so there is no
# frame object to take a type from. Expose the name as a distinct type with
# CPython's exact name; `isinstance`-style checks then answer False instead
# of the import dying with AttributeError. Real frame objects (and their
# type) come with the traceback work.
FrameType = type('frame', (), {})

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

class DynamicClassAttribute:
    """Route attribute access on a class to __getattr__.

    This is a descriptor, used to define attributes that act differently
    when accessed through an instance and through a class.  Instance access
    remains normal, but access to an attribute through a class will be
    routed to the class's __getattr__ method; this is done by raising
    AttributeError.

    This allows one to have properties active on an instance, and have
    virtual attributes on the class with the same name.  (Enum used this
    between Python versions 3.4 - 3.9 .)

    Subclass from this to use a different method of accessing virtual
    attributes and still be treated properly by the inspect module.  (Enum
    uses this since Python 3.10 .)

    """
    def __init__(self, fget=None, fset=None, fdel=None, doc=None):
        self.fget = fget
        self.fset = fset
        self.fdel = fdel
        # next two lines make DynamicClassAttribute act the same as property
        self.__doc__ = doc or fget.__doc__
        self.overwrite_doc = doc is None
        # support for abstract methods
        self.__isabstractmethod__ = bool(getattr(fget, '__isabstractmethod__', False))

    def __get__(self, instance, ownerclass=None):
        if instance is None:
            if self.__isabstractmethod__:
                return self
            raise AttributeError()
        elif self.fget is None:
            raise AttributeError("unreadable attribute")
        return self.fget(instance)

    def __set__(self, instance, value):
        if self.fset is None:
            raise AttributeError("can't set attribute")
        self.fset(instance, value)

    def __delete__(self, instance):
        if self.fdel is None:
            raise AttributeError("can't delete attribute")
        self.fdel(instance)

    def getter(self, fget):
        fdoc = fget.__doc__ if self.overwrite_doc else None
        result = type(self)(fget, self.fset, self.fdel, fdoc or self.__doc__)
        result.overwrite_doc = self.overwrite_doc
        return result

    def setter(self, fset):
        result = type(self)(self.fget, fset, self.fdel, self.__doc__)
        result.overwrite_doc = self.overwrite_doc
        return result

    def deleter(self, fdel):
        result = type(self)(self.fget, self.fset, fdel, self.__doc__)
        result.overwrite_doc = self.overwrite_doc
        return result


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
