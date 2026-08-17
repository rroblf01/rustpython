"""Comprehensive typing stub for RustPython - uses instances for subscriptable types."""

TYPE_CHECKING = False

class _GenericAlias:
    """Support Type[X] subscript syntax via instance __getitem__."""
    def __init__(self, origin, args):
        self.__origin__ = origin
        self.__args__ = args
    def __repr__(self):
        if not isinstance(self.__args__, tuple):
            self.__args__ = (self.__args__,)
        return '%s[%s]' % (self.__origin__.__name__, ', '.join(str(a) for a in self.__args__))

class _TypingType:
    """Typing types are singletons that support X[Y] via __getitem__, and callable for TypeVar etc."""
    def __init__(self, name):
        self._name = name
    def __getitem__(self, item):
        return _GenericAlias(self, item)
    def __call__(self, *args, **kwargs):
        if self._name == 'TypeVar':
            return object()
        if self._name == 'NamedTuple':
            return type('NamedTuple', (), {})
        if self._name == 'NewType':
            def _newtype(name, tp):
                return tp
            return _newtype
        return None
    def __repr__(self):
        return self._name

Any = _TypingType('Any')
Awaitable = _TypingType('Awaitable')
Callable = _TypingType('Callable')
Coroutine = _TypingType('Coroutine')
Generic = _TypingType('Generic')
Optional = _TypingType('Optional')
TypeVar = _TypingType('TypeVar')
Union = _TypingType('Union')
Dict = _TypingType('Dict')
List = _TypingType('List')
Set = _TypingType('Set')
FrozenSet = _TypingType('FrozenSet')
Tuple = _TypingType('Tuple')
Iterable = _TypingType('Iterable')
Iterator = _TypingType('Iterator')
Sequence = _TypingType('Sequence')
Mapping = _TypingType('Mapping')
MutableMapping = _TypingType('MutableMapping')
Generator = _TypingType('Generator')
AsyncGenerator = _TypingType('AsyncGenerator')
AsyncIterable = _TypingType('AsyncIterable')
AsyncIterator = _TypingType('AsyncIterator')

ParamSpec = _TypingType('ParamSpec')
Protocol = _TypingType('Protocol')
Literal = _TypingType('Literal')
TypedDict = _TypingType('TypedDict')
ClassVar = _TypingType('ClassVar')
Final = _TypingType('Final')
Self = _TypingType('Self')
NoReturn = _TypingType('NoReturn')
Never = NoReturn
NamedTuple = _TypingType('NamedTuple')
NewType = _TypingType('NewType')
# PEP 646 (variadic generics) / PEP 692 (`**kwargs: Unpack[...]`) — both
# missing entirely, so any code merely IMPORTING one of these (not even
# using it meaningfully) failed at collection time with `ImportError`.
# Real trigger: CPython's own `test_annotationlib.py`'s
# `from typing import Unpack, ...`. Stubbed the same way every other
# subscriptable typing construct here is (a `_TypingType` singleton
# supporting `X[...]`), matching this module's existing "good enough to
# import and subscript, not full runtime semantics" scope.
TypeVarTuple = _TypingType('TypeVarTuple')
Unpack = _TypingType('Unpack')

def overload(func): return func
def cast(typ, val): return val
def type_check_only(func): return func
def no_type_check(func=None):
    """Decorator to indicate that a function should not be type-checked."""
    if func is None:
        return lambda f: f
    return func

# CPython 3.11+ sentinel values
class _NoDefault:
    """Sentinel for default-argument removal."""
    def __repr__(self): return 'typing.NoDefault'
    def __reduce__(self): return (type(self), ())
NoDefault = _NoDefault()

# CPython 3.12+ TypeAliasType
class TypeAliasType:
    """A type alias type, created by type_statement."""
    def __init__(self, name, value, *, type_params=()):
        self.__value__ = value
        self.__type_params__ = type_params
        self.__name__ = name
    def __getitem__(self, item):
        if not isinstance(item, tuple):
            item = (item,)
        return self.__origin__._replace_args(item) if hasattr(self, '__origin__') else _GenericAlias(self, item)
    def __repr__(self):
        return self.__name__

# Additional stubs needed by tests
TypeGuard = _TypingType('TypeGuard')
TypeIs = _TypingType('TypeIs')
TypeAlias = _TypingType('TypeAlias')

# PEP 616+ type string literals
AnyStr = _TypingType('AnyStr')

# Additional type stubs needed by tests
T = TypeVar('T')
T_co = TypeVar('T_co')
KT = TypeVar('KT')
VT = TypeVar('VT')
VT_co = TypeVar('VT_co')
T_contra = TypeVar('T_contra')
F = TypeVar('F')
P = ParamSpec('P')

def override(func): return func
def assert_type(val, typ): return val
def assert_never(val): pass
def reveal_type(val): return val
def assert_never(val): pass

def dataclass_transform(*, factory_default_sentinel=None, eq_default=True, order_default=True, kw_only_default=False, field_specifiers=(), **kwargs):
    """Decorator for dataclass transforms."""
    def decorator(cls_or_func):
        return cls_or_func
    if factory_default_sentinel is not None:
        return decorator
    return decorator

def get_args(tp):
    """Get type arguments from a generic type."""
    if hasattr(tp, '__args__'):
        return tp.__args__
    return ()

def get_origin(tp):
    """Get the origin of a generic type."""
    if hasattr(tp, '__origin__'):
        return tp.__origin__
    return None

def get_overloads(func):
    """Get overloaded implementations of a function."""
    return getattr(func, '__overloaded__', ())

def clear_overloads():
    """Clear all overloaded implementations."""
    pass

def get_type_hints(obj, globalns=None, localns=None, include_extras=False):
    """Minimal stub: real semantics need the full PEP 563/649 evaluation
    machinery this module doesn't implement — just exposes __annotations__
    (or {} if absent), which is enough for code that merely checks presence/
    keys rather than relying on forward-ref resolution."""
    return dict(getattr(obj, '__annotations__', {}) or {})

import collections
OrderedDict = collections.OrderedDict

class _ProtocolMeta(type):
    """Metaclass for the structural `Supports*` protocols: isinstance()
    checks for the required methods via __subclasshook__."""
    def __instancecheck__(cls, inst):
        return cls.__subclasshook__(type(inst))
    def __subclasscheck__(cls, sub):
        return cls.__subclasshook__(sub)

def _check_methods(C, *methods):
    mro = C.__mro__
    for method in methods:
        for B in mro:
            if method in B.__dict__:
                if B.__dict__[method] is None:
                    return NotImplemented
                break
        else:
            return NotImplemented
    return True

class SupportsInt(metaclass=_ProtocolMeta):
    @classmethod
    def __subclasshook__(cls, C):
        if cls is SupportsInt:
            return _check_methods(C, '__int__')
        return NotImplemented

class SupportsFloat(metaclass=_ProtocolMeta):
    @classmethod
    def __subclasshook__(cls, C):
        if cls is SupportsFloat:
            return _check_methods(C, '__float__')
        return NotImplemented

class SupportsComplex(metaclass=_ProtocolMeta):
    @classmethod
    def __subclasshook__(cls, C):
        if cls is SupportsComplex:
            return _check_methods(C, '__complex__')
        return NotImplemented

class SupportsRound(metaclass=_ProtocolMeta):
    @classmethod
    def __subclasshook__(cls, C):
        if cls is SupportsRound:
            return _check_methods(C, '__round__')
        return NotImplemented

class SupportsIndex(metaclass=_ProtocolMeta):
    @classmethod
    def __subclasshook__(cls, C):
        if cls is SupportsIndex:
            return _check_methods(C, '__index__')
        return NotImplemented

class SupportsAbs(metaclass=_ProtocolMeta):
    @classmethod
    def __subclasshook__(cls, C):
        if cls is SupportsAbs:
            return _check_methods(C, '__abs__')
        return NotImplemented

__all__ = [
    'TYPE_CHECKING', 'Any', 'Awaitable', 'Callable', 'Coroutine',
    'Generic', 'Optional', 'TypeVar', 'Union', 'Dict', 'List',
    'Set', 'FrozenSet', 'Tuple', 'Iterable', 'Iterator', 'Sequence',
    'Mapping', 'MutableMapping', 'Generator', 'ParamSpec', 'Protocol',
    'Literal', 'TypedDict', 'ClassVar', 'Final', 'Self', 'overload',
    'cast', 'NoReturn', 'NamedTuple', 'NewType', 'TypeVarTuple', 'Unpack',
    'SupportsInt', 'SupportsFloat', 'SupportsComplex', 'SupportsRound',
    'SupportsIndex', 'SupportsAbs',
    'get_type_hints',
]
