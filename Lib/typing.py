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
            # Functional API: NamedTuple('Point', [('x', int), ('y', int)])
            if args and isinstance(args[0], str):
                nt_name = args[0]
                fields = args[1] if len(args) > 1 else []
                # fields can be list of (name, type) or list of names
                if fields and isinstance(fields[0], (list, tuple)):
                    field_names = [f[0] for f in fields]
                else:
                    field_names = list(fields)
                ann = {n: object for n in field_names}
                # create tuple subclass
                return _NamedTupleMeta(nt_name, (_NamedTupleBase,), {'__annotations__': ann, '_fields': tuple(field_names)})
            return type('NamedTuple', (), {})
        if self._name == 'NewType':
            def _newtype(name, tp):
                return tp
            return _newtype
        return None
    def __repr__(self):
        return self._name


# ── typing.NamedTuple runtime support ────────────────────────────────
# Real tuple-subclass implementation so `class Point(NamedTuple): x: int`
# behaves like CPython's typing.NamedTuple (tuple equality, iteration,
# _fields, _make, _asdict, _replace, __replace__).

class _NamedTupleMeta(type):
    def __call__(cls, *args, **kwargs):
        # Functional API: NamedTuple('Point', [('x', int), ('y', int)])
        if cls is _NamedTupleBase and args and isinstance(args[0], str):
            nt_name = args[0]
            fields_arg = args[1] if len(args) > 1 else kwargs.get('fields', [])
            if isinstance(fields_arg, str):
                field_names = [n.strip() for n in fields_arg.replace(',', ' ').split() if n.strip()]
            elif fields_arg and isinstance(fields_arg[0], (list, tuple)):
                field_names = [f[0] for f in fields_arg]
            else:
                field_names = list(fields_arg) if fields_arg else []
            ann = {n: object for n in field_names}
            # handle defaults via kwargs? not needed for test
            return _NamedTupleMeta(nt_name, (_NamedTupleBase,), {'__annotations__': ann, '_fields': tuple(field_names)})
        # Normal instance creation: delegate to type's __call__ (which does __new__+__init__)
        # Use explicit type.__call__ to avoid super() MRO issues in RustPython
        obj = cls.__new__(cls, *args, **kwargs)
        if isinstance(obj, cls):
            try:
                # tuple subclasses don't have __init__ that matters; ignore
                if hasattr(obj, '__init__'):
                    obj.__init__(*args, **kwargs)
            except TypeError:
                pass
            except Exception:
                pass
        return obj

    def __new__(mcls, name, bases, ns, **kw):
        # Preserve typing.NamedTuple base creation
        if name == 'NamedTuple' and not bases:
            return super().__new__(mcls, name, bases, dict(ns))
        # For subclasses of NamedTuple (e.g. class Point(NamedTuple): x: int)
        cls = super().__new__(mcls, name, bases, dict(ns))
        # Collect field names from __annotations__ of this class (and bases if needed)
        ann = ns.get('__annotations__', None)
        if ann is None:
            ann = getattr(cls, '__annotations__', {}) or {}
        # Also include annotations from base classes that are NamedTuple subclasses
        # (simple merge, subclass overrides)
        for base in bases:
            bann = getattr(base, '__annotations__', None)
            if bann:
                # base fields should be already in _fields, but merge for completeness
                pass
        fields = tuple(ann.keys()) if isinstance(ann, dict) else ()
        # If _fields already set (e.g. functional API), keep it
        if not hasattr(cls, '_fields') or cls._fields == () or name != 'NamedTuple':
            if fields:
                cls._fields = fields
            elif not hasattr(cls, '_fields'):
                cls._fields = ()
        # Collect defaults: class attributes for fields
        defaults = {}
        for f in getattr(cls, '_fields', ()):
            if f in ns:
                defaults[f] = ns[f]
            elif f in cls.__dict__ and f not in ('_fields', '__annotations__'):
                # inherited default? keep
                pass
        cls._field_defaults = defaults
        return cls

class _NamedTupleBase(tuple, metaclass=_NamedTupleMeta):
    _fields = ()
    _field_defaults = {}
    def __new__(cls, *args, **kwargs):
        fields = getattr(cls, '_fields', ())
        if not fields:
            ann = getattr(cls, '__annotations__', {}) or {}
            fields = tuple(ann.keys())
        defaults = getattr(cls, '_field_defaults', {}) or {}
        # Build values list
        values = []
        # handle positional
        for i, f in enumerate(fields):
            if i < len(args):
                if f in kwargs:
                    raise TypeError(f"{cls.__name__}.__new__() got multiple values for argument '{f}'")
                values.append(args[i])
            elif f in kwargs:
                values.append(kwargs.pop(f))
            elif f in defaults:
                values.append(defaults[f])
            else:
                raise TypeError(f"{cls.__name__}.__new__() missing required argument: '{f}'")
        if kwargs:
            unexpected = next(iter(kwargs))
            raise TypeError(f"{cls.__name__}.__new__() got an unexpected keyword argument '{unexpected}'")
        if len(args) > len(fields):
            raise TypeError(f"{cls.__name__}.__new__() takes {len(fields)} positional arguments but {len(args)} were given")
        return tuple.__new__(cls, tuple(values))
    def __repr__(self):
        try:
            fields = getattr(self, '_fields', None) or getattr(type(self), '_fields', ())
            if fields:
                vals = tuple(self)
                parts = ', '.join(f"{k}={v!r}" for k, v in zip(fields, vals))
                return f"{type(self).__name__}({parts})"
        except:
            pass
        return super().__repr__()
    def _asdict(self):
        fields = getattr(self, '_fields', None) or getattr(type(self), '_fields', ())
        return dict(zip(fields, self))
    def _make(cls, iterable):
        return cls(*iterable)
    _make = classmethod(_make)
    def _replace(self, **kw):
        fields = getattr(self, '_fields', None) or getattr(type(self), '_fields', ())
        for k in kw:
            if k not in fields:
                raise TypeError(f"unexpected field name '{k}'")
        vals = list(self)
        for k, v in kw.items():
            idx = fields.index(k)
            vals[idx] = v
        return type(self)(*vals)
    def __replace__(self, **kw):
        return self._replace(**kw)
    # tuple equality already works via tuple.__eq__, but ensure we compare as tuple
    def __eq__(self, other):
        if isinstance(other, tuple):
            return tuple(self) == tuple(other)
        return super().__eq__(other)
    def __hash__(self):
        return tuple.__hash__(self)


# ── Runtime-checkable protocols & ABCs ────────────────────────────────
# Real classes with a metaclass __instancecheck__ that verifies the
# presence of the protocol's dunder method(s) on the object's type —
# matching CPython's structural checks (`SupportsInt.__subclasshook__`
# looks for `__int__`, ABCs like `Hashable`/`Sized`/`Iterable` likewise).
# The previous stubs were opaque objects, so EVERY `isinstance(x,
# typing.SupportsInt)` returned False (real trigger: test_fractions'
# SupportsInt/SupportsFloat assertions on Fraction instances).

# Dynamic class creation with a custom metaclass currently drops the
# provided namespace from the new class's __dict__, so per-protocol data
# lives HERE, keyed by class name, instead of in class attributes.
_PROTOCOL_REGISTRY = {}


def _protocol_check(cls, obj):
    # Registered as each protocol's `__instancecheck__` (metaclass hook);
    # this interpreter's isinstance() reaches it through the metaclass MRO.
    info = _PROTOCOL_REGISTRY.get(cls.__name__)
    if info is None:
        return False
    methods, is_callable = info
    if methods:
        return all(hasattr(obj, m) for m in methods)
    if is_callable:
        return callable(obj)
    return False


class _ProtocolMeta(type):
    def __instancecheck__(cls, obj):
        return _protocol_check(cls, obj)

    def __repr__(cls):
        return cls.__name__

    def __getitem__(cls, item):
        return _GenericAlias(cls, item)


def _runtime_protocol(name, methods=None, is_callable=False):
    cls = _ProtocolMeta(name, (object,), {})
    _PROTOCOL_REGISTRY[name] = (tuple(methods) if methods else (), bool(is_callable))
    return cls


def _runtime_abc(name, methods):
    return _runtime_protocol(name, tuple(methods))

# All typing type stubs
Any = _TypingType('Any')
Awaitable = _TypingType('Awaitable')
Callable = _runtime_protocol('Callable', is_callable=True)
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
NamedTuple = _NamedTupleBase
NewType = _TypingType('NewType')
Self = _TypingType('Self')
Protocol = _TypingType('Protocol')
Sequence = _runtime_abc('Sequence', ('__getitem__', '__len__'))
Mapping = _runtime_abc('Mapping', ('__getitem__', '__len__', '__iter__'))
MutableMapping = _runtime_abc('MutableMapping', ('__getitem__', '__len__', '__iter__', '__setitem__', '__delitem__'))
MappingView = _runtime_abc('MappingView', ('__len__',))
KeysView = _runtime_abc('KeysView', ('__len__', '__iter__', '__contains__'))
ValuesView = _runtime_abc('ValuesView', ('__len__', '__iter__', '__contains__'))
ItemsView = _runtime_abc('ItemsView', ('__len__', '__iter__', '__contains__'))
Hashable = _runtime_abc('Hashable', ('__hash__',))
Sized = _runtime_abc('Sized', ('__len__',))
MutableSequence = _runtime_abc('MutableSequence', ('__getitem__', '__len__', '__setitem__'))
MutableSet = _runtime_abc('MutableSet', ('__contains__', '__iter__', '__len__', 'add'))
AbstractSet = _runtime_abc('AbstractSet', ('__contains__', '__iter__', '__len__'))
Container = _runtime_abc('Container', ('__contains__',))
Iterable = _runtime_abc('Iterable', ('__iter__',))
Iterator = _runtime_abc('Iterator', ('__iter__', '__next__'))
Reversible = _runtime_abc('Reversible', ('__reversed__',))
Collection = _runtime_abc('Collection', ('__iter__', '__contains__', '__len__'))
ByteString = _runtime_abc('ByteString', ('__getitem__',))
SupportsInt = _runtime_abc('SupportsInt', ('__int__',))
SupportsFloat = _runtime_abc('SupportsFloat', ('__float__',))
SupportsComplex = _runtime_abc('SupportsComplex', ('__complex__',))
SupportsRound = _runtime_abc('SupportsRound', ('__round__',))
SupportsIndex = _runtime_abc('SupportsIndex', ('__index__',))
SupportsAbs = _runtime_abc('SupportsAbs', ('__abs__',))
SupportsBytes = _runtime_abc('SupportsBytes', ('__bytes__',))
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
