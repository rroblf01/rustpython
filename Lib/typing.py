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
NamedTuple = _TypingType('NamedTuple')
NewType = _TypingType('NewType')

def overload(func): return func
def cast(typ, val): return val
def type_check_only(func): return func

import collections
OrderedDict = collections.OrderedDict

__all__ = [
    'TYPE_CHECKING', 'Any', 'Awaitable', 'Callable', 'Coroutine',
    'Generic', 'Optional', 'TypeVar', 'Union', 'Dict', 'List',
    'Set', 'FrozenSet', 'Tuple', 'Iterable', 'Iterator', 'Sequence',
    'Mapping', 'MutableMapping', 'Generator', 'ParamSpec', 'Protocol',
    'Literal', 'TypedDict', 'ClassVar', 'Final', 'Self', 'overload',
    'cast', 'NoReturn', 'NamedTuple', 'NewType',
]
