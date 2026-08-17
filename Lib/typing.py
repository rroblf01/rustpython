"""Comprehensive typing stub for RustPython - uses instances for subscriptable types."""

TYPE_CHECKING = False

class _GenericAlias:
    def __init__(self, origin, args):
        self.__origin__ = origin
        self.__args__ = args
    def __repr__(self):
        if not isinstance(self.__args__, tuple):
            self.__args__ = (self.__args__,)
        return '%s[%s]' % (self.__origin__.__name__, ', '.join(str(a) for a in self.__args__))

class _TypingType:
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

# All typing type stubs
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
Generator = _TypingType('Generator')
AsyncGenerator = _TypingType('AsyncGenerator')
AsyncIterable = _TypingType('AsyncIterable')
AsyncIterator = _TypingType('AsyncIterator')
Type = _TypingType('Type')
ReadOnly = _TypingType('ReadOnly')
ClassVar = _TypingType('ClassVar')
Final = _TypingType('Final')
final = Final
Annotated = _TypingType('Annotated')
Concatenate = _TypingType('Concatenate')
Required = _TypingType('Required')
NotRequired = _TypingType('NotRequired')
IO = _TypingType('IO')
TextIO = _TypingType('TextIO')
BinaryIO = _TypingType('BinaryIO')
Pattern = _TypingType('Pattern')
Match = _TypingType('Match')
Counter = _TypingType('Counter')
ChainMap = _TypingType('ChainMap')
Deque = _TypingType('Deque')
DefaultDict = _TypingType('DefaultDict')
ForwardRef = _TypingType('ForwardRef')
Unpack = _TypingType('Unpack')
TypeVarTuple = _TypingType('TypeVarTuple')
ParamSpec = _TypingType('ParamSpec')
ParamSpecArgs = _TypingType('ParamSpecArgs')
ParamSpecKwargs = _TypingType('ParamSpecKwargs')
Literal = _TypingType('Literal')
TypedDict = _TypingType('TypedDict')
NoReturn = _TypingType('NoReturn')
Never = NoReturn
NamedTuple = _TypingType('NamedTuple')
NewType = _TypingType('NewType')
Self = _TypingType('Self')
Protocol = _TypingType('Protocol')
Sequence = _TypingType('Sequence')
Mapping = _TypingType('Mapping')
MutableMapping = _TypingType('MutableMapping')
MappingView = _TypingType('MappingView')
KeysView = _TypingType('KeysView')
ValuesView = _TypingType('ValuesView')
ItemsView = _TypingType('ItemsView')
Reversible = _TypingType('Reversible')
Collection = _TypingType('Collection')
Container = _TypingType('Container')
Hashable = _TypingType('Hashable')
Sized = _TypingType('Sized')
MutableSequence = _TypingType('MutableSequence')
MutableSet = _TypingType('MutableSet')
AbstractSet = _TypingType('AbstractSet')
ByteString = _TypingType('ByteString')
SupportsInt = _TypingType('SupportsInt')
SupportsFloat = _TypingType('SupportsFloat')
SupportsComplex = _TypingType('SupportsComplex')
SupportsRound = _TypingType('SupportsRound')
SupportsIndex = _TypingType('SupportsIndex')
SupportsAbs = _TypingType('SupportsAbs')
LiteralString = _TypingType('LiteralString')
TypeGuard = _TypingType('TypeGuard')
TypeIs = _TypingType('TypeIs')
TypeAlias = _TypingType('TypeAlias')
AnyStr = _TypingType('AnyStr')

# TypeVar instances
T = TypeVar('T')
T_co = TypeVar('T_co')
KT = TypeVar('KT')
VT = TypeVar('VT')
VT_co = TypeVar('VT_co')
T_contra = TypeVar('T_contra')
F = TypeVar('F')
P = ParamSpec('P')

# CPython 3.11+ sentinel
class _NoDefault:
    def __repr__(self): return 'typing.NoDefault'
    def __reduce__(self): return (type(self), ())
NoDefault = _NoDefault()

class TypeAliasType:
    def __init__(self, name, value, *, type_params=()):
        self.__value__ = value
        self.__type_params__ = type_params
        self.__name__ = name
    def __getitem__(self, item):
        if not isinstance(item, tuple):
            item = (item,)
        return _GenericAlias(self, item)
    def __repr__(self):
        return self.__name__

def overload(func): return func
def cast(typ, val): return val
def type_check_only(func): return func
def no_type_check(func=None):
    if func is None: return lambda f: f
    return func
no_type_check_decorator = no_type_check
def override(func): return func
def runtime_checkable(cls): return cls
def assert_type(val, typ): return val
def assert_never(val): pass
def reveal_type(val): return val
def get_args(tp):
    return tp.__args__ if hasattr(tp, '__args__') else ()
def get_origin(tp):
    return tp.__origin__ if hasattr(tp, '__origin__') else None
def get_overloads(func):
    return getattr(func, '__overloaded__', ())
def clear_overloads(): pass
def get_protocol_members(cls): return set()
def is_typeddict(tp):
    return hasattr(tp, '__annotations__') and hasattr(tp, '__required_keys__')
def is_protocol(tp):
    return getattr(tp, '_is_protocol', False)
def is_type_alias(tp):
    return isinstance(tp, TypeAliasType)
def get_type_hints(obj, globalns=None, localns=None, include_extras=False):
    return dict(getattr(obj, '__annotations__', {}) or {})
def dataclass_transform(*args, **kwargs):
    def deco(f): return f
    return deco if args else deco

# Private attrs used by test_typing
_cleanups = []
_ASSERT_NEVER_REPR_MAX_LENGTH = 100
_overload_registry = {}
def _get_protocol_attrs(cls): return set()
def _eval_type(t, g, l): return t
def _collect_parameters(args): return set()
class SupportsBytes: pass

# collections.abc re-exports
from _collections_abc import (
    Iterable, Iterator, Generator, Coroutine,
    AsyncIterable, AsyncIterator, AsyncGenerator,
    Callable, Container, Hashable, Sized,
    Sequence, Reversible, MutableSequence, MutableMapping, MutableSet,
    MappingView, KeysView, ValuesView, ItemsView,
)
try:
    from collections import Counter, ChainMap, Deque, DefaultDict
except (ImportError, AttributeError):
    pass
import collections
OrderedDict = collections.OrderedDict
class ContextManager:
    def __enter__(self): return self
    def __exit__(self, *a): pass
class AsyncContextManager:
    def __aenter__(self): pass
    def __aexit__(self, *a): pass
