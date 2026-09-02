# Access WeakSet through the weakref module.
# This code is separated-out because it is needed
# by abc.py to load everything else at startup.

from _weakref import ref
from types import GenericAlias

__all__ = ['WeakSet']


class WeakSet:
    __slots__ = ('data', '_remove', '__weakref__', '__dict__')
    def __init__(self, data=None):
        self.data = set()

        def _remove(item, selfref=ref(self)):
            self = selfref()
            if self is not None:
                self.data.discard(item)

        self._remove = _remove
        if data is not None:
            self.update(data)

    def __iter__(self):
        for itemref in self.data.copy():
            item = itemref()
            if item is not None:
                # Caveat: the iterator will keep a strong reference to
                # `item` until it is resumed or closed.
                yield item

    def __len__(self):
        return len(self.data)

    def __contains__(self, item):
        try:
            wr = ref(item)
        except TypeError:
            return False
        return wr in self.data

    def __reduce__(self):
        # Don't double-pickle data (already in args as list(self))
        # Handle both __dict__ and __slots__
        state = {}
        if hasattr(self, '__dict__'):
            state.update({k: v for k, v in self.__dict__.items() if k != 'data'})
        for cls in type(self).__mro__:
            if '__slots__' in cls.__dict__:
                slots = cls.__dict__['__slots__']
                if isinstance(slots, str):
                    slots = (slots,)
                for slot in slots:
                    if slot in ('__weakref__', '__dict__', 'data'):
                        continue
                    if hasattr(self, slot):
                        try:
                            state[slot] = getattr(self, slot)
                        except AttributeError:
                            pass
        return self.__class__, (list(self),), state

    def __getstate__(self):
        state = {}
        if hasattr(self, '__dict__'):
            state.update({k: v for k, v in self.__dict__.items() if k not in ('data', '__weakref__', '__dict__')})
        for cls in type(self).__mro__:
            if '__slots__' in cls.__dict__:
                slots = cls.__dict__['__slots__']
                if isinstance(slots, str):
                    slots = (slots,)
                for slot in slots:
                    if slot in ('__weakref__', '__dict__', 'data'):
                        continue
                    if hasattr(self, slot):
                        try:
                            state[slot] = getattr(self, slot)
                        except AttributeError:
                            pass
        return state

    def __setstate__(self, state):
        for k, v in state.items():
            if k in ('__weakref__', '__dict__', 'data'):
                continue
            setattr(self, k, v)

    def __deepcopy__(self, memo):
        from copy import deepcopy
        cls = self.__class__
        result = cls.__new__(cls)
        memo[id(self)] = result
        result.data = set()
        # _remove needs to be set before adding items
        result._remove = self._remove
        # Copy state
        state = self.__getstate__()
        for k, v in state.items():
            setattr(result, k, deepcopy(v, memo))
        # Add items via weakrefs
        for item in self:
            result.add(item)
        return result

    def add(self, item):
        self.data.add(ref(item, self._remove))

    def clear(self):
        self.data.clear()

    def copy(self):
        return self.__class__(self)

    def pop(self):
        while True:
            try:
                itemref = self.data.pop()
            except KeyError:
                raise KeyError('pop from empty WeakSet') from None
            item = itemref()
            if item is not None:
                return item

    def remove(self, item):
        self.data.remove(ref(item))

    def discard(self, item):
        try:
            wr = ref(item)
        except TypeError:
            raise TypeError(f"cannot create weak reference to '{type(item).__name__}' object")
        self.data.discard(wr)

    def update(self, other):
        for element in other:
            self.add(element)

    def __ior__(self, other):
        self.update(other)
        return self

    def difference(self, other):
        newset = self.copy()
        newset.difference_update(other)
        return newset
    __sub__ = difference

    def difference_update(self, other):
        self.__isub__(other)
    def __isub__(self, other):
        if self is other:
            self.data.clear()
        else:
            self.data.difference_update(ref(item) for item in other)
        return self

    def intersection(self, other):
        return self.__class__(item for item in other if item in self)
    __and__ = intersection

    def intersection_update(self, other):
        self.__iand__(other)
    def __iand__(self, other):
        self.data.intersection_update(ref(item) for item in other)
        return self

    def issubset(self, other):
        return self.data.issubset(ref(item) for item in other)
    __le__ = issubset

    def __lt__(self, other):
        return self.data < set(map(ref, other))

    def issuperset(self, other):
        return self.data.issuperset(ref(item) for item in other)
    __ge__ = issuperset

    def __gt__(self, other):
        return self.data > set(map(ref, other))

    def __eq__(self, other):
        if not isinstance(other, self.__class__):
            return NotImplemented
        return self.data == set(map(ref, other))

    def symmetric_difference(self, other):
        newset = self.copy()
        newset.symmetric_difference_update(other)
        return newset
    __xor__ = symmetric_difference

    def symmetric_difference_update(self, other):
        self.__ixor__(other)
    def __ixor__(self, other):
        if self is other:
            self.data.clear()
        else:
            self.data.symmetric_difference_update(ref(item, self._remove) for item in other)
        return self

    def union(self, other):
        result = self.copy()
        result.update(other)
        return result
    __or__ = union

    def isdisjoint(self, other):
        return len(self.intersection(other)) == 0

    def __repr__(self):
        return repr(self.data)

    __class_getitem__ = classmethod(GenericAlias)
