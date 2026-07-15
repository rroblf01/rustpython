# typing_extensions stub for RustPython
# Re-exports common typing_extensions names that Django and friends need.

# Type constructors as plain classes (not true generics, but enough for imports)
Literal = type('Literal', (), {})
TypedDict = type('TypedDict', (), {})
Protocol = type('Protocol', (), {})

# Runtime-checkable decorator
def runtime_checkable(cls):
    return cls

# ClassVar, Final, etc.
ClassVar = type('ClassVar', (), {})
Final = type('Final', (), {})

# Annotated — used by modern Django type hints
def Annotated(*args, **kwargs):
    if len(args) >= 1:
        return args[0]
    return object

# TypeAlias
TypeAlias = type('TypeAlias', (), {})

# Self — Python 3.11+
Self = type('Self', (), {})

# Required/NotRequired — for TypedDict
Required = type('Required', (), {})
NotRequired = type('NotRequired', (), {})
