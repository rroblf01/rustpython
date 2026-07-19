class UserList:
    def __init__(self, initlist=None):
        self.data = []
        if initlist is not None:
            if isinstance(initlist, list):
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
        return self.data[i]

    def __setitem__(self, i, item):
        self.data[i] = item

    def __delitem__(self, i):
        del self.data[i]

    def __add__(self, other):
        if isinstance(other, UserList):
            return self.__class__(self.data + other.data)
        elif isinstance(other, list):
            return self.__class__(self.data + other)
        return self.__class__(self.data + list(other))

    def __radd__(self, other):
        if isinstance(other, UserList):
            return self.__class__(other.data + self.data)
        elif isinstance(other, list):
            return self.__class__(other + self.data)
        return self.__class__(list(other) + self.data)

    def __iadd__(self, other):
        if isinstance(other, UserList):
            self.data += other.data
        elif isinstance(other, list):
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

    def __iter__(self):
        return iter(self.data)

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
        return self.__class__(self.data)

    def count(self, item):
        return self.data.count(item)

    def index(self, item):
        return self.data.index(item)

    def reverse(self):
        self.data.reverse()

    def sort(self):
        self.data.sort()

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
        return self.__class__(self.data)


class Counter(dict):
    def __init__(self, iterable=None, **kwds):
        super().__init__()
        self.update(iterable, **kwds)

    def __missing__(self, key):
        return 0

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

    def update(self, iterable=None, **kwds):
        if iterable is not None:
            if hasattr(iterable, 'keys'):
                for elem in iterable:
                    self[elem] = self.get(elem, 0) + iterable[elem]
            else:
                for elem in iterable:
                    self[elem] = self.get(elem, 0) + 1
        if kwds:
            self.update(kwds)

    def subtract(self, iterable=None, **kwds):
        if iterable is not None:
            if hasattr(iterable, 'keys'):
                for elem in iterable:
                    self[elem] = self.get(elem, 0) - iterable[elem]
            else:
                for elem in iterable:
                    self[elem] = self.get(elem, 0) - 1
        if kwds:
            self.subtract(kwds)

    def total(self):
        return sum(self.values())

    def copy(self):
        return Counter(self)

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
        return self

    def __isub__(self, other):
        for elem, count in other.items():
            self[elem] = self.get(elem, 0) - count
        return self


class defaultdict(dict):
    def __init__(self, default_factory=None, *args, **kwargs):
        self.default_factory = default_factory
        if args or kwargs:
            self.update(*args, **kwargs)

    def __missing__(self, key):
        if self.default_factory is None:
            raise KeyError(key)
        value = self.default_factory()
        self[key] = value
        return value

    def __repr__(self):
        items = ', '.join('%r: %r' % (k, v) for k, v in self.items())
        return 'defaultdict(%r, {%s})' % (self.default_factory, items)

    def copy(self):
        result = defaultdict(self.default_factory)
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

    def strip(self):
        return self.__class__(self.data.strip())

    def split(self, sep=None):
        return self.data.split(sep)

    def join(self, seq):
        return self.__class__(self.data.join(seq))

    def replace(self, old, new):
        if isinstance(old, UserString):
            old = old.data
        if isinstance(new, UserString):
            new = new.data
        return self.__class__(self.data.replace(old, new))

    def startswith(self, prefix):
        return self.data.startswith(prefix)

    def endswith(self, suffix):
        return self.data.endswith(suffix)
