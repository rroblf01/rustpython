def _count_elements(mapping, iterable):
    """CPython's C-accelerated `_collections._count_elements`: count each
    element of `iterable` into `mapping`, in place."""
    for elem in iterable:
        mapping[elem] = mapping.get(elem, 0) + 1

class UserList:
    def __init__(self, initlist=None):
        self.data = []
        if initlist is not None:
            if type(initlist) == type(self.data):
                self.data[:] = initlist
            elif isinstance(initlist, UserList):
                self.data[:] = initlist.data[:]
            else:
                self.data = list(initlist)

    def __repr__(self):
        return repr(self.data)

    def __lt__(self, other):
        return self.data < self.__cast(other)

    def __le__(self, other):
        return self.data <= self.__cast(other)

    def __eq__(self, other):
        return self.data == self.__cast(other)

    def __gt__(self, other):
        return self.data > self.__cast(other)

    def __ge__(self, other):
        return self.data >= self.__cast(other)

    def __cast(self, other):
        return other.data if isinstance(other, UserList) else other

    def __contains__(self, item):
        return item in self.data

    def __len__(self):
        return len(self.data)

    def __getitem__(self, i):
        if isinstance(i, slice):
            return self.__class__(self.data[i])
        else:
            return self.data[i]

    def __iter__(self):
        i = 0
        while True:
            try:
                yield self[i]
            except IndexError:
                return
            i += 1

    def __setitem__(self, i, item):
        self.data[i] = item

    def __delitem__(self, i):
        del self.data[i]

    def __add__(self, other):
        if isinstance(other, UserList):
            return self.__class__(self.data + other.data)
        elif isinstance(other, type(self.data)):
            return self.__class__(self.data + other)
        return self.__class__(self.data + list(other))

    def __radd__(self, other):
        if isinstance(other, UserList):
            return self.__class__(other.data + self.data)
        elif isinstance(other, type(self.data)):
            return self.__class__(other + self.data)
        return self.__class__(list(other) + self.data)

    def __iadd__(self, other):
        if isinstance(other, UserList):
            self.data += other.data
        elif isinstance(other, type(self.data)):
            self.data += other
        else:
            self.data += list(other)
        return self

    def __mul__(self, n):
        return self.__class__(self.data * n)

    __rmul__ = __mul__

    def __imul__(self, n):
        self.data *= n
        return self

    def __copy__(self):
        inst = self.__class__.__new__(self.__class__)
        inst.__dict__.update(self.__dict__)
        inst.__dict__["data"] = self.__dict__["data"][:]
        return inst

    def append(self, item):
        self.data.append(item)

    def insert(self, i, item):
        self.data.insert(i, item)

    def pop(self, i=-1):
        return self.data.pop(i)

    def remove(self, item):
        self.data.remove(item)

    def clear(self):
        self.data.clear()

    def copy(self):
        return self.__class__(self)

    def count(self, item):
        return self.data.count(item)

    def index(self, item, *args):
        return self.data.index(item, *args)

    def reverse(self):
        self.data.reverse()

    def sort(self, *args, **kwds):
        self.data.sort(*args, **kwds)

    def extend(self, other):
        if isinstance(other, UserList):
            self.data.extend(other.data)
        else:
            self.data.extend(other)


class UserDict:
    def __init__(self, initdata=None, **kwargs):
        self.data = {}
        if initdata is not None:
            self.update(initdata)
        if kwargs:
            self.update(kwargs)

    def __len__(self):
        return len(self.data)

    def __getitem__(self, key):
        if key in self.data:
            return self.data[key]
        raise KeyError(key)

    def __setitem__(self, key, item):
        self.data[key] = item

    def __delitem__(self, key):
        del self.data[key]

    def __iter__(self):
        return iter(self.data)

    def __contains__(self, key):
        return key in self.data

    def __repr__(self):
        return repr(self.data)

    def __eq__(self, other):
        if isinstance(other, UserDict):
            return self.data == other.data
        return self.data == other

    def get(self, key, default=None):
        return self.data.get(key, default)

    def keys(self):
        return self.data.keys()

    def values(self):
        return self.data.values()

    def items(self):
        return self.data.items()

    def pop(self, key, default=None):
        return self.data.pop(key, default)

    def popitem(self):
        return self.data.popitem()

    def clear(self):
        self.data.clear()

    def setdefault(self, key, default=None):
        return self.data.setdefault(key, default)

    def update(self, other=None, **kwargs):
        if other is not None:
            if isinstance(other, UserDict):
                self.data.update(other.data)
            elif hasattr(other, 'keys'):
                for k in other.keys():
                    self.data[k] = other[k]
            else:
                for k, v in other:
                    self.data[k] = v
        if kwargs:
            self.data.update(kwargs)

    def copy(self):
        if self.__class__ is UserDict:
            return UserDict(self.data.copy())
        import copy as _copy
        new = self.__class__.__new__(self.__class__)
        new.__dict__.update(self.__dict__)
        new.data = self.data.copy()
        return new

    def __copy__(self):
        return self.copy()

    def __deepcopy__(self, memo):
        import copy as _copy
        new = self.__class__.__new__(self.__class__)
        memo[id(self)] = new
        new.__dict__.update(_copy.deepcopy(self.__dict__, memo))
        return new

    @classmethod
    def fromkeys(cls, iterable, value=None):
        return cls(dict.fromkeys(iterable, value))


class Counter(dict):
    def __init__(self, iterable=None, /, **kwds):
        # additive, not clearing
        self.update(iterable, **kwds)

    def __missing__(self, key):
        return 0

    @classmethod
    def fromkeys(cls, iterable, v=None):
        raise NotImplementedError(
            'Counter.fromkeys() is undefined.  Use Counter(iterable) instead.')

    def __eq__(self, other):
        if not isinstance(other, Counter):
            return NotImplemented
        return all(self[e] == other[e] for c in (self, other) for e in c)

    def __le__(self, other):
        if not isinstance(other, Counter):
            return NotImplemented
        return all(self[e] <= other[e] for c in (self, other) for e in c)

    def __lt__(self, other):
        if not isinstance(other, Counter):
            return NotImplemented
        return self <= other and self != other

    def __ge__(self, other):
        if not isinstance(other, Counter):
            return NotImplemented
        return all(self[e] >= other[e] for c in (self, other) for e in c)

    def __gt__(self, other):
        if not isinstance(other, Counter):
            return NotImplemented
        return self >= other and self != other

    def most_common(self, n=None):
        items = list(self.items())
        items.sort(key=lambda kv: kv[1], reverse=True)
        if n is None:
            return items
        return items[:n]

    def elements(self):
        result = []
        for elem, count in self.items():
            i = 0
            while i < count:
                result.append(elem)
                i += 1
        return iter(result)

    def update(self, iterable=None, /, **kwds):
        if iterable is not None:
            if hasattr(iterable, 'keys'):
                if self:
                    self_get = self.get
                    for elem, count in iterable.items():
                        self[elem] = count + self_get(elem, 0)
                else:
                    super().update(iterable)
            else:
                _count_elements(self, iterable)
        if kwds:
            self.update(kwds)

    def subtract(self, iterable=None, /, **kwds):
        if iterable is not None:
            if hasattr(iterable, 'keys'):
                if self:
                    self_get = self.get
                    for elem, count in iterable.items():
                        self[elem] = self_get(elem, 0) - count
                else:
                    super().update({k: -v for k, v in iterable.items()})
            else:
                for elem in iterable:
                    self[elem] = self.get(elem, 0) - 1
        if kwds:
            self.subtract(kwds)

    def total(self):
        return sum(self.values())

    def copy(self):
        return self.__class__(self)

    def __delitem__(self, elem):
        if elem in self:
            super().__delitem__(elem)

    def __repr__(self):
        if not self:
            return 'Counter()'
        items = ', '.join('%r: %r' % pair for pair in self.most_common())
        return 'Counter({%s})' % items

    def __add__(self, other):
        result = Counter()
        for elem, count in self.items():
            newcount = count + other.get(elem, 0)
            if newcount > 0:
                result[elem] = newcount
        for elem, count in other.items():
            if elem not in self and count > 0:
                result[elem] = count
        return result

    def __sub__(self, other):
        result = Counter()
        for elem, count in self.items():
            newcount = count - other.get(elem, 0)
            if newcount > 0:
                result[elem] = newcount
        for elem, count in other.items():
            if elem not in self and count < 0:
                result[elem] = 0 - count
        return result

    def __or__(self, other):
        result = Counter()
        for elem, count in self.items():
            other_count = other.get(elem, 0)
            newcount = other_count if count < other_count else count
            if newcount > 0:
                result[elem] = newcount
        for elem, count in other.items():
            if elem not in self and count > 0:
                result[elem] = count
        return result

    def __and__(self, other):
        result = Counter()
        for elem, count in self.items():
            other_count = other.get(elem, 0)
            newcount = count if count < other_count else other_count
            if newcount > 0:
                result[elem] = newcount
        return result

    def __pos__(self):
        result = Counter()
        for elem, count in self.items():
            if count > 0:
                result[elem] = count
        return result

    def __neg__(self):
        result = Counter()
        for elem, count in self.items():
            if count < 0:
                result[elem] = 0 - count
        return result

    def __iadd__(self, other):
        for elem, count in other.items():
            self[elem] = self.get(elem, 0) + count
        to_del = []
        for k, v in list(self.items()):
            if v <= 0:
                to_del.append(k)
        for k in to_del:
            del self[k]
        return self

    def __isub__(self, other):
        for elem, count in other.items():
            self[elem] = self.get(elem, 0) - count
        to_del = []
        for k, v in list(self.items()):
            if v <= 0:
                to_del.append(k)
        for k in to_del:
            del self[k]
        return self

    def __ior__(self, other):
        for elem, count in other.items():
            newcount = count if self.get(elem, 0) < count else self.get(elem, 0)
            if newcount > 0:
                self[elem] = newcount
            elif elem in self:
                del self[elem]
        to_del = []
        for k, v in list(self.items()):
            if v <= 0:
                to_del.append(k)
        for k in to_del:
            del self[k]
        return self

    def __iand__(self, other):
        for elem in list(self.keys()):
            if elem in other:
                newcount = self[elem] if self[elem] < other[elem] else other[elem]
                if newcount > 0:
                    self[elem] = newcount
                else:
                    del self[elem]
            else:
                del self[elem]
        return self

    def __ixor__(self, other):
        for elem, count in list(self.items()):
            self[elem] = abs(count - other.get(elem, 0))
        for elem, count in other.items():
            if elem not in self:
                ac = abs(count)
                if ac:
                    self[elem] = ac
        to_del = []
        for k, v in list(self.items()):
            if v <= 0:
                to_del.append(k)
        for k in to_del:
            del self[k]
        return self

    def __xor__(self, other):
        if not isinstance(other, Counter):
            return NotImplemented
        result = Counter()
        for elem, count in self.items():
            newcount = abs(count - other.get(elem, 0))
            if newcount:
                result[elem] = newcount
        for elem, count in other.items():
            if elem not in self and count:
                result[elem] = abs(count)
        return result


_defaultdict_repr_guard = set()


class defaultdict(dict):
    def __init__(self, default_factory=None, *args, **kwargs):
        if default_factory is not None and not callable(default_factory):
            raise TypeError('first argument must be callable or None')
        self.default_factory = default_factory
        if args or kwargs:
            self.update(*args, **kwargs)

    def __missing__(self, key):
        if self.default_factory is None:
            raise KeyError(key)
        value = self.default_factory()
        # Don't clobber a value a deeper (reentrant) __missing__ frame
        # already stored for this key — CPython's dict storage order makes
        # the innermost result win, which test_factory_conflict... asserts.
        if key not in self:
            self[key] = value
        return self[key]

    def __repr__(self):
        # Recursion guard (gh-145492): a factory whose __repr__ calls
        # repr(dd) must not recurse forever -- CPython's Py_ReprEnter
        # returns the standard '...' cycle marker instead.
        key = id(self)
        if key in _defaultdict_repr_guard:
            # CPython dict-subclass recursion marker keeps the items part.
            items = ', '.join('%r: %r' % (k, v) for k, v in self.items())
            return '%s(..., {%s})' % (type(self).__name__, items)
        _defaultdict_repr_guard.add(key)
        try:
            items = ', '.join('%r: %r' % (k, v) for k, v in self.items())
            return '%s(%r, {%s})' % (type(self).__name__, self.default_factory, items)
        finally:
            _defaultdict_repr_guard.discard(key)

    def copy(self):
        result = defaultdict(self.default_factory)
        result.update(self)
        return result

    def __reduce__(self):
        # CPython-style reduce: (class, args, state, listitems, dictitems).
        # The factory is pickled by reference (int, None, callables...).
        return (
            self.__class__,
            (self.default_factory,),
            None,
            None,
            iter(self.items()),
        )

    @staticmethod
    def _is_dict_like(o):
        # Our isinstance() doesn't currently resolve native bases through
        # Python-level subclasses, so probe structurally. Lists/tuples of
        # (k, v) pairs are accepted too (dict |= items-list semantics).
        if isinstance(o, dict):
            return True
        return hasattr(o, "keys") and hasattr(o, "__getitem__")

    def __ior__(self, other):
        if isinstance(other, (list, tuple)):
            for k, v in other:
                self[k] = v
            return self
        # Mapping: route through the native backing's update via the same
        # mechanism dict.update(self, other) would use, but our native base
        # doesn't expose update as an unbound callable — so iterate manually.
        if hasattr(other, "keys"):
            for k in other.keys():
                self[k] = other[k]
        else:
            for k, v in other:
                self[k] = v
        return self

    def __or__(self, other):
        if not self._is_dict_like(other):
            # Raise directly: our binary-op dispatch doesn't convert
            # NotImplemented to TypeError like real CPython's slot
            # machinery does.
            raise TypeError(
                "unsupported operand type(s) for |: '%s' and '%s'"
                % (type(self).__name__, type(other).__name__)
            )
        result = defaultdict(self.default_factory, self)
        result.update(other)
        return result

    def __ror__(self, other):
        if not self._is_dict_like(other):
            raise TypeError(
                "unsupported operand type(s) for |: '%s' and '%s'"
                % (type(other).__name__, type(self).__name__)
            )
        result = defaultdict(self.default_factory, other)
        result.update(self)
        return result


class UserString:
    def __init__(self, seq):
        if isinstance(seq, str):
            self.data = seq
        elif isinstance(seq, UserString):
            self.data = seq.data
        else:
            self.data = str(seq)

    def __str__(self):
        return str(self.data)

    def __repr__(self):
        return repr(self.data)

    def __int__(self):
        return int(self.data)

    def __float__(self):
        return float(self.data)

    def __len__(self):
        return len(self.data)

    def __getitem__(self, index):
        return self.__class__(self.data[index])

    def __eq__(self, other):
        if isinstance(other, UserString):
            return self.data == other.data
        return self.data == other

    def __lt__(self, other):
        if isinstance(other, UserString):
            return self.data < other.data
        return self.data < other

    def __le__(self, other):
        if isinstance(other, UserString):
            return self.data <= other.data
        return self.data <= other

    def __gt__(self, other):
        if isinstance(other, UserString):
            return self.data > other.data
        return self.data > other

    def __ge__(self, other):
        if isinstance(other, UserString):
            return self.data >= other.data
        return self.data >= other

    def __contains__(self, char):
        if isinstance(char, UserString):
            char = char.data
        return char in self.data

    def __iter__(self):
        return iter(self.data)

    def __hash__(self):
        return hash(self.data)

    def __add__(self, other):
        if isinstance(other, UserString):
            return self.__class__(self.data + other.data)
        return self.__class__(self.data + str(other))

    def __radd__(self, other):
        if isinstance(other, UserString):
            return self.__class__(other.data + self.data)
        return self.__class__(str(other) + self.data)

    def __mul__(self, n):
        return self.__class__(self.data * n)

    __rmul__ = __mul__

    def upper(self):
        return self.__class__(self.data.upper())

    def lower(self):
        return self.__class__(self.data.lower())

    def strip(self, *args, **kwargs):
        return self.__class__(self.data.strip(*args, **kwargs))

    def lstrip(self, *args, **kwargs):
        return self.__class__(self.data.lstrip(*args, **kwargs))

    def rstrip(self, *args, **kwargs):
        return self.__class__(self.data.rstrip(*args, **kwargs))

    def _convert_args(self, args):
        """Convert UserString arguments to strings for delegation."""
        converted = []
        for arg in args:
            if isinstance(arg, UserString):
                converted.append(arg.data)
            elif isinstance(arg, bytes):
                converted.append(arg.decode('latin-1'))
            else:
                converted.append(arg)
        return tuple(converted)

    def find(self, *args, **kwargs):
        return self.data.find(*self._convert_args(args), **kwargs)

    def rfind(self, *args, **kwargs):
        return self.data.rfind(*self._convert_args(args), **kwargs)

    def index(self, *args, **kwargs):
        return self.data.index(*self._convert_args(args), **kwargs)

    def rindex(self, *args, **kwargs):
        return self.data.rindex(*self._convert_args(args), **kwargs)

    def count(self, *args, **kwargs):
        return self.data.count(*self._convert_args(args), **kwargs)

    def startswith(self, *args, **kwargs):
        return self.data.startswith(*self._convert_args(args), **kwargs)

    def endswith(self, *args, **kwargs):
        return self.data.endswith(*self._convert_args(args), **kwargs)

    def replace(self, *args, **kwargs):
        return self.__class__(self.data.replace(*self._convert_args(args), **kwargs))

    def format(self, *args, **kwargs):
        return self.__class__(self.data.format(*args, **kwargs))

    def format_map(self, mapping):
        return self.__class__(self.data.format_map(mapping))

    def title(self):
        return self.__class__(self.data.title())

    def capitalize(self):
        return self.__class__(self.data.capitalize())

    def swapcase(self):
        return self.__class__(self.data.swapcase())

    def casefold(self):
        return self.__class__(self.data.casefold())

    def center(self, *args, **kwargs):
        return self.__class__(self.data.center(*args, **kwargs))

    def ljust(self, *args, **kwargs):
        return self.__class__(self.data.ljust(*args, **kwargs))

    def rjust(self, *args, **kwargs):
        return self.__class__(self.data.rjust(*args, **kwargs))

    def zfill(self, *args, **kwargs):
        return self.__class__(self.data.zfill(*args, **kwargs))

    def expandtabs(self, *args, **kwargs):
        return self.__class__(self.data.expandtabs(*args, **kwargs))

    def encode(self, *args, **kwargs):
        return self.data.encode(*args, **kwargs)

    def decode(self, *args, **kwargs):
        return self.data.decode(*args, **kwargs)

    def splitlines(self, *args, **kwargs):
        return self.data.splitlines(*args, **kwargs)

    def partition(self, *args, **kwargs):
        return self.data.partition(*args, **kwargs)

    def rpartition(self, *args, **kwargs):
        return self.data.rpartition(*args, **kwargs)

    def rsplit(self, *args, **kwargs):
        return self.data.rsplit(*args, **kwargs)

    def isalpha(self):
        return self.data.isalpha()

    def isdigit(self):
        return self.data.isdigit()

    def isalnum(self):
        return self.data.isalnum()

    def isspace(self):
        return self.data.isspace()

    def isupper(self):
        return self.data.isupper()

    def islower(self):
        return self.data.islower()

    def istitle(self):
        return self.data.istitle()

    def isascii(self):
        return self.data.isascii()

    def removesuffix(self, *args, **kwargs):
        return self.__class__(self.data.removesuffix(*args, **kwargs))

    def removeprefix(self, *args, **kwargs):
        return self.__class__(self.data.removeprefix(*args, **kwargs))

    def __mod__(self, other):
        return self.__class__(self.data % other)

    def __rmod__(self, other):
        return self.__class__(other % self.data)

    def split(self, *args, **kwargs):
        return self.data.split(*args, **kwargs)

    def join(self, seq):
        return self.__class__(self.data.join(seq))

    @staticmethod
    def maketrans(*args, **kwargs):
        return str.maketrans(*args, **kwargs)


class ChainMap:
    """A ChainMap groups multiple dicts (or other mappings) together
    to create a single, updateable view.

    The underlying mappings are stored in a list. Lookups search the
    underlying mappings successively until a key is found; writes, updates,
    and deletions only operate on the first mapping.

    Standalone (doesn't subclass `_collections_abc.MutableMapping`, unlike
    real CPython) since this interpreter's `abc`/mixin machinery is still a
    stub — every dict-like method real ChainMap gets for free from that
    mixin is implemented directly here instead.
    """

    def __init__(self, *maps):
        self.maps = list(maps) or [{}]

    def __missing__(self, key):
        raise KeyError(key)

    def __getitem__(self, key):
        for mapping in self.maps:
            try:
                return mapping[key]
            except KeyError:
                pass
        return self.__missing__(key)

    def get(self, key, default=None):
        for m in self.maps:
            try:
                try:
                    contains = key in m
                except Exception:
                    try:
                        m[key]
                        contains = True
                    except KeyError:
                        contains = False
                    except Exception:
                        contains = False
                if contains:
                    return m[key]
            except KeyError:
                pass
            except Exception:
                pass
        return default

    def __len__(self):
        return len(set().union(*self.maps))

    def __iter__(self):
        d = {}
        for mapping in reversed(self.maps):
            d.update(dict.fromkeys(mapping))
        return iter(d)

    def __contains__(self, key):
        for m in self.maps:
            try:
                if key in m:
                    return True
            except Exception:
                try:
                    m[key]
                    return True
                except KeyError:
                    pass
                except Exception:
                    pass
                continue
        return False

    def __or__(self, other):
        # Subclass priority: if other is a ChainMap subclass that defines its own __ror__,
        # return NotImplemented so Python will try other.__ror__(self) which will
        # correctly return the subclass type (test_union_operators expects
        # ChainMap() | SubclassRor() to be SubclassRor, not ChainMap).
        if isinstance(other, ChainMap):
            other_type = type(other)
            if other_type is not ChainMap:
                # Check if other_type defines __ror__ directly (not just inherited)
                try:
                    if "__ror__" in other_type.__dict__:
                        return NotImplemented
                except Exception:
                    pass
                # Also check via MRO: if other_type is subclass and has __ror__ in its own dict
                # we already handled; otherwise, for plain Subclass without __ror__, we should
                # handle via normal path (return ChainMap type), so don't return NotImplemented
                # for plain Subclass.
                # To distinguish, check if other_type has __ror__ that is not ChainMap's
                try:
                    if hasattr(other_type, "__ror__") and other_type.__dict__.get("__ror__") is not None:
                        # Has own __ror__, let it handle
                        return NotImplemented
                except Exception:
                    pass
        try:
            from collections.abc import Mapping as _MappingABC
            is_mapping = isinstance(other, _MappingABC)
        except Exception:
            is_mapping = hasattr(other, 'keys') and hasattr(other, '__getitem__')
        if isinstance(other, ChainMap):
            other = dict(other)
            is_mapping = True
        if is_mapping:
            if isinstance(other, dict):
                new_first = self.maps[0].copy()
                for k, v in other.items():
                    new_first[k] = v
                return self.__class__(new_first, *self.maps[1:])
            new_first = self.maps[0].copy()
            try:
                for k in other.keys():
                    new_first[k] = other[k]
            except Exception:
                return NotImplemented
            return self.__class__(new_first, *self.maps[1:])
        return NotImplemented

    def __ror__(self, other):
        if isinstance(other, ChainMap):
            other = dict(other)
        if isinstance(other, dict):
            merged = other.copy()
            merged.update(dict(self))
            return self.__class__(merged)
        if hasattr(other, 'keys'):
            merged = dict(other)
            merged.update(dict(self))
            return self.__class__(merged)
        return NotImplemented

    def __ior__(self, other):
        if isinstance(other, ChainMap):
            other = dict(other)
        if isinstance(other, dict):
            for k, v in other.items():
                self.maps[0][k] = v
            return self
        if hasattr(other, 'keys'):
            for k in other.keys():
                self.maps[0][k] = other[k]
            return self
        try:
            for k, v in other:
                self.maps[0][k] = v
            return self
        except Exception:
            return NotImplemented

    def __copy__(self):
        return self.__class__(self.maps[0].copy(), *self.maps[1:])

    def __deepcopy__(self, memo):
        from copy import deepcopy
        return self.__class__(*[deepcopy(m, memo) for m in self.maps])

    def __reduce__(self):
        return (self.__class__, tuple(self.maps))

    def __bool__(self):
        return any(self.maps)

    def __repr__(self):
        maps_repr = ", ".join(repr(m) for m in self.maps)
        return f"{self.__class__.__name__}({maps_repr})"

    @classmethod
    def fromkeys(cls, iterable, *args):
        return cls(dict.fromkeys(iterable, *args))

    def copy(self):
        return self.__class__(self.maps[0].copy(), *self.maps[1:])

    __copy__ = copy

    def new_child(self, m=None, **kwargs):
        if m is None:
            m = kwargs
        elif kwargs:
            m.update(kwargs)
        return self.__class__(m, *self.maps)

    @property
    def parents(self):
        return self.__class__(*self.maps[1:])

    def __setitem__(self, key, value):
        self.maps[0][key] = value

    def __delitem__(self, key):
        try:
            del self.maps[0][key]
        except KeyError:
            raise KeyError(f'Key not found in the first mapping: {key!r}')

    def popitem(self):
        try:
            return self.maps[0].popitem()
        except KeyError:
            raise KeyError('No keys found in the first mapping.')

    def pop(self, key, *args):
        try:
            return self.maps[0].pop(key, *args)
        except KeyError:
            raise KeyError(f'Key not found in the first mapping: {key!r}')

    def clear(self):
        self.maps[0].clear()

    def keys(self):
        return list(self.__iter__())

    def values(self):
        return [self[k] for k in self]

    def items(self):
        return [(k, self[k]) for k in self]

    def __eq__(self, other):
        if isinstance(other, ChainMap):
            return dict(self) == dict(other)
        return NotImplemented

    def update(self, *args, **kwargs):
        if args:
            other = args[0]
            if hasattr(other, "keys"):
                for k in other.keys():
                    self.maps[0][k] = other[k]
            else:
                for k, v in other:
                    self.maps[0][k] = v
        for k, v in kwargs.items():
            self.maps[0][k] = v

    def setdefault(self, key, default=None):
        if key not in self:
            self.maps[0][key] = default
        return self[key]
