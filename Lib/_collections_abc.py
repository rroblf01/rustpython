"""Abstract Base Classes for containers (Mapping, Sequence, etc.)."""

class Mapping:
    """Base class for mapping-like objects."""
    def __getitem__(self, key):
        raise NotImplementedError
    def __iter__(self):
        raise NotImplementedError
    def __len__(self):
        raise NotImplementedError
    def __contains__(self, key):
        try:
            self[key]
        except KeyError:
            return False
        except TypeError:
            return False
        return True
    def keys(self):
        return KeysView(self)
    def values(self):
        return ValuesView(self)
    def items(self):
        return ItemsView(self)
    def get(self, key, default=None):
        try:
            return self[key]
        except KeyError:
            return default
        except TypeError:
            return default
    def __eq__(self, other):
        if not isinstance(other, Mapping):
            return NotImplemented
        if len(self) != len(other):
            return False
        for key in self:
            try:
                if self[key] != other[key]:
                    return False
            except KeyError:
                return False
        return True
    def __ne__(self, other):
        result = self.__eq__(other)
        if result is NotImplemented:
            return result
        return not result

class KeysView:
    def __init__(self, mapping):
        self._mapping = mapping
    def __iter__(self):
        return iter(self._mapping)
    def __len__(self):
        return len(self._mapping)
    def __contains__(self, key):
        return key in self._mapping
    def __repr__(self):
        return f"dict_keys({list(self._mapping)})"

class ValuesView:
    def __init__(self, mapping):
        self._mapping = mapping
    def __iter__(self):
        for key in self._mapping:
            yield self._mapping[key]
    def __len__(self):
        return len(self._mapping)
    def __repr__(self):
        return f"dict_values({list(self._mapping.values())})"

class ItemsView:
    def __init__(self, mapping):
        self._mapping = mapping
    def __iter__(self):
        for key in self._mapping:
            yield (key, self._mapping[key])
    def __len__(self):
        return len(self._mapping)
    def __contains__(self, item):
        key, value = item
        try:
            return self._mapping[key] == value
        except KeyError:
            return False
    def __repr__(self):
        return f"dict_items({list(self._mapping.items())})"

class Sequence:
    """Base class for sequence-like objects."""
    def __getitem__(self, index):
        raise NotImplementedError
    def __len__(self):
        raise NotImplementedError
    def __contains__(self, value):
        for i in range(len(self)):
            try:
                if self[i] == value:
                    return True
            except (IndexError, TypeError):
                pass
        return False
    def __iter__(self):
        i = 0
        try:
            while True:
                v = self[i]
                yield v
                i += 1
        except IndexError:
            return
    def index(self, value):
        for i in range(len(self)):
            if self[i] == value:
                return i
        raise ValueError(f"{value} is not in sequence")
    def count(self, value):
        return sum(1 for v in self if v == value)

class MutableSequence(Sequence):
    def __setitem__(self, index, value):
        raise NotImplementedError
    def __delitem__(self, index):
        raise NotImplementedError
    def append(self, value):
        self.insert(len(self), value)
    def insert(self, index, value):
        raise NotImplementedError
    def reverse(self):
        i, j = 0, len(self) - 1
        while i < j:
            self[i], self[j] = self[j], self[i]
            i += 1
            j -= 1
    def sort(self, *, key=None, reverse=False):
        pass

class Iterable:
    def __iter__(self):
        raise NotImplementedError

class Iterator(Iterable):
    def __next__(self):
        raise NotImplementedError
    def __iter__(self):
        return self

class Generator(Iterator):
    def __next__(self):
        raise NotImplementedError
    def __iter__(self):
        return self
    def send(self, value):
        raise NotImplementedError
    def throw(self, type, value=None, traceback=None):
        raise NotImplementedError
    @property
    def gi_frame(self):
        return None
    @property
    def gi_running(self):
        return False

class Awaitable:
    def __await__(self):
        raise NotImplementedError

class Coroutine(Awaitable):
    def send(self, value):
        raise NotImplementedError
    def throw(self, type, value=None, traceback=None):
        raise NotImplementedError
    def close(self):
        pass

class AsyncIterable:
    def __aiter__(self):
        raise NotImplementedError

class AsyncIterator(AsyncIterable):
    def __anext__(self):
        raise NotImplementedError
    def __aiter__(self):
        return self

class AsyncGenerator(AsyncIterator):
    def __anext__(self):
        raise NotImplementedError
    def __aiter__(self):
        return self
    def asend(self, value):
        raise NotImplementedError
    def athrow(self, type, value=None, traceback=None):
        raise NotImplementedError
    def aclose(self):
        pass

class Callable:
    def __call__(self, *args, **kwargs):
        raise NotImplementedError
