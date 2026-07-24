"""Helper to provide extensibility for pickle.

This is only useful to add pickle support for extension types defined in
C, not for instances of user-defined classes.
"""

__all__ = ["pickle", "constructor",
           "add_extension", "remove_extension", "clear_extension_cache"]

dispatch_table = {}


def pickle(ob_type, pickle_function, constructor_ob=None):
    if not callable(pickle_function):
        raise TypeError("reduction functions must be callable")
    dispatch_table[ob_type] = pickle_function

    if constructor_ob is not None:
        constructor(constructor_ob)


def constructor(object):
    if not callable(object):
        raise TypeError("constructors must be callable")


def pickle_complex(c):
    return complex, (c.real, c.imag)


try:
    pickle(complex, pickle_complex, complex)
except NameError:
    # This interpreter has no `complex` builtin yet.
    pass


def _reconstructor(cls, base, state):
    if base is object:
        obj = object.__new__(cls)
    else:
        obj = base.__new__(cls, state)
    if base.__init__ != object.__init__:
        base.__init__(obj, state)
    return obj


def __newobj__(cls, *args):
    return cls.__new__(cls, *args)


def __newobj_ex__(cls, args, kwargs):
    """Used by pickle protocol 4, instead of __newobj__ to allow classes with
    keyword-only arguments to be pickled correctly.
    """
    return cls.__new__(cls, *args, **kwargs)


def _slotnames(cls):
    """Return a list of slot names for a given class."""
    names = cls.__dict__.get("__slotnames__")
    if names is not None:
        return names

    names = []
    if hasattr(cls, "__slots__"):
        for c in cls.__mro__:
            if "__slots__" in c.__dict__:
                slots = c.__dict__["__slots__"]
                if isinstance(slots, str):
                    slots = (slots,)
                for name in slots:
                    if name in ("__dict__", "__weakref__"):
                        continue
                    elif name.startswith("__") and not name.endswith("__"):
                        stripped = c.__name__.lstrip("_")
                        if stripped:
                            names.append("_%s%s" % (stripped, name))
                        else:
                            names.append(name)
                    else:
                        names.append(name)

    try:
        cls.__slotnames__ = names
    except Exception:
        pass

    return names


# A registry of extension codes.  This is not only used to reduce the size
# of pickles, but also to find the module and function that pickle needs
# to import to unpickle an object.
_extension_registry = {}
_inverted_registry = {}
_extension_cache = {}


def add_extension(module, name, code):
    """Register an extension code."""
    code = int(code)
    if not 1 <= code <= 0x7fffffff:
        raise ValueError("code out of range")
    key = (module, name)
    if (_extension_registry.get(key) == code and
            _inverted_registry.get(code) == key):
        return
    if key in _extension_registry:
        raise ValueError("key %s is already registered with code %s" %
                          (key, _extension_registry[key]))
    if code in _inverted_registry:
        raise ValueError("code %s is already in use for key %s" %
                          (code, _inverted_registry[code]))
    _extension_registry[key] = code
    _inverted_registry[code] = key


def remove_extension(module, name, code):
    """Unregister an extension code.  For testing only."""
    key = (module, name)
    if (_extension_registry.get(key) != code or
            _inverted_registry.get(code) != key):
        raise ValueError("key %s is not registered with code %s" %
                          (key, code))
    del _extension_registry[key]
    del _inverted_registry[code]


def clear_extension_cache():
    _extension_cache.clear()
