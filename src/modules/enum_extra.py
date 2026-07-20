"""Real `enum` module implementation (Enum/IntEnum/StrEnum/EnumType/auto/
unique/nonmember), loaded as Python source via
VirtualMachine::install_source_defined_stdlib — see that function's doc
comment for why. Deliberately scoped to what's needed for real-world use
(including Django's `Choices`/`IntegerChoices`/`TextChoices`, which build a
custom metaclass on top of `EnumType`): no Flag/IntFlag, no functional API
(`Enum('Name', ['A', 'B'])`), no `__prepare__`-based duplicate-name
detection (this interpreter's class namespace is already delivered here in
definition order — see Frame::name_order / PyDict::order — which is the
part CPython's `_EnumDict` needs `__prepare__` for; strict duplicate
rejection is not).
"""


class auto:
    """Sentinel: `_generate_next_value_` fills in the real value at class
    creation time."""

    def __init__(self):
        self.value = None


class nonmember:
    """Wraps a class-body value so EnumType.__new__ treats it as a plain
    attribute, never a member (e.g. Django's Choices uses this for
    `do_not_call_in_templates`)."""

    def __init__(self, value):
        self.value = value


class member:
    """Inverse of nonmember(): force a value (e.g. one that would
    otherwise look like a descriptor) to be treated as a member."""

    def __init__(self, value):
        self.value = value


# Re-exported as enum.property — a plain @property already gets skipped by
# EnumType.__new__'s "descriptors are never members" rule, so no special
# behavior is needed beyond the alias Django (and other real code) imports
# it under.
property = property


def _is_descriptor(value):
    return isinstance(value, (property, staticmethod, classmethod))


def _is_sunder_or_dunder(name):
    return len(name) > 1 and name[0] == "_" and name[-1] == "_"


def _generate_next_value(name, start, count, last_values):
    return count


def _is_member_candidate(key, value):
    """Shared classification rule: would this class-body assignment become
    an enum member? Used both by `_EnumDict.__setitem__` (tracking member
    names as the class body assigns them — what a metaclass built on top of
    EnumType, e.g. Django's `ChoicesType`, inspects directly via
    `classdict._member_names`) and by `EnumType.__new__`'s own fallback
    scan (used only if `__prepare__` didn't run for some reason)."""
    if _is_sunder_or_dunder(key):
        return False
    if isinstance(value, nonmember):
        return False
    if isinstance(value, member):
        return True
    if _is_descriptor(value) or callable(value):
        return False
    return True


class _EnumDict(dict):
    """The namespace object `EnumType.__prepare__` hands back — a real dict
    subclass (reusing this interpreter's native dict-subclassing support)
    so class-body assignments can be tracked in definition order via a
    plain instance attribute (`_member_names`) as they happen, exactly like
    CPython's own enum module needs `__prepare__` for. Real code
    (Django's `ChoicesType.__new__`) reads `_member_names` directly."""

    def __init__(self):
        super().__init__()
        self._member_names = []

    def __setitem__(self, key, value):
        if _is_member_candidate(key, value) and key not in self._member_names:
            self._member_names.append(key)
        super().__setitem__(key, value)


class EnumType(type):
    @classmethod
    def __prepare__(metacls, name, bases, **kwds):
        return _EnumDict()

    def __new__(metacls, name, bases, namespace, **kwds):
        member_names = getattr(namespace, "_member_names", None)
        if member_names is None:
            # No __prepare__-provided _EnumDict (shouldn't normally happen
            # now that EnumType always supplies one) — fall back to
            # scanning the plain namespace directly.
            member_names = [k for k in namespace.keys() if _is_member_candidate(k, namespace[k])]

        raw_values = {}
        for key in member_names:
            value = namespace[key]
            if isinstance(value, member):
                value = value.value
            raw_values[key] = value
            del namespace[key]
        # Any nonmember()-wrapped values still need unwrapping before the
        # class body's own namespace is handed to super().__new__ — member
        # candidates were already removed above, so this only touches the
        # non-member remainder. Uses `dict.__setitem__` (bypassing
        # `_EnumDict.__setitem__`) deliberately: a plain `namespace[key] =
        # value.value` would re-run the member-candidate classification on
        # the now-unwrapped value, which no longer looks like a
        # `nonmember(...)` and would get *reclassified* as a real member —
        # this is exactly why Django's own `ChoicesType.__new__` uses
        # `dict.__setitem__(classdict, key, value)` for its own in-place
        # rewrite of already-classified values instead of plain subscript
        # assignment.
        for key in list(namespace.keys()):
            value = namespace[key]
            if isinstance(value, nonmember):
                dict.__setitem__(namespace, key, value.value)

        cls = super().__new__(metacls, name, bases, namespace, **kwds)

        # Looked up on the now-constructed `cls` (not `namespace`, which
        # only holds this class's own body) so an override inherited from a
        # base — e.g. StrEnum's `_generate_next_value_` turning auto() into
        # a lowercased name, needed by `TextChoices(Choices, StrEnum)`,
        # which doesn't redefine it itself — is actually found via the
        # normal mro instead of always falling back to the plain-Enum
        # default.
        generate_next_value = getattr(cls, "_generate_next_value_", _generate_next_value)
        resolved_values = {}
        last_values = []
        for key in member_names:
            value = raw_values[key]
            if isinstance(value, auto):
                value = generate_next_value(key, 1, len(last_values) + 1, list(last_values))
            elif isinstance(value, tuple) and len(value) == 1:
                # A single-element tuple value is CPython enum's convention
                # for "this is really just one plain value" (real Enum
                # unpacks a member's tuple value as *args to the mixin
                # type's __new__, and a single-arg tuple degenerates to
                # that one arg) — needed for Django's `IntegerChoices`/
                # `TextChoices`, whose `ChoicesType.__new__` strips the
                # trailing label out of a `(value, label)` pair and passes
                # the remaining `(value,)` through here.
                value = value[0]
            resolved_values[key] = value
            last_values.append(value)

        member_map = {}
        value2member = {}
        cls._member_names_ = []
        cls._member_map_ = member_map
        cls._value2member_map_ = value2member
        for key in member_names:
            value = resolved_values[key]
            existing = None
            for mname in cls._member_names_:
                mv = member_map[mname]._value_
                if mv == value or mv is value:
                    existing = mname
                    break
            if existing is not None:
                alias = member_map[existing]
                member_map[key] = alias
                setattr(cls, key, alias)
                continue
            # Always pass `value` through to object.__new__ — whether it
            # actually becomes the instance's native backing depends on
            # whether `cls` transparently subclasses a native type
            # (int/str/...), which object.__new__ (Rust side) already knows
            # how to check on `cls` itself (propagated down from IntEnum/
            # StrEnum regardless of how many `bases` levels away that mixin
            # was introduced); a plain Enum subclass just ignores the extra
            # arg and builds a bare instance, same as before.
            instance = object.__new__(cls, value)
            instance._name_ = key
            instance._value_ = value
            cls._member_names_.append(key)
            member_map[key] = instance
            try:
                value2member[value] = instance
            except TypeError:
                pass
            setattr(cls, key, instance)
        return cls

    def __iter__(cls):
        return iter([cls._member_map_[n] for n in cls._member_names_])

    def __len__(cls):
        return len(cls._member_names_)

    def __reversed__(cls):
        return iter([cls._member_map_[n] for n in reversed(cls._member_names_)])

    def __contains__(cls, value):
        if isinstance(value, cls):
            return True
        return value in cls._value2member_map_

    def __getitem__(cls, name):
        return cls._member_map_[name]

    def __call__(cls, value, *args):
        if not args and isinstance(value, cls):
            return value
        try:
            return cls._value2member_map_[value]
        except (KeyError, TypeError):
            for m in cls:
                if m._value_ == value:
                    return m
            raise ValueError(f"{value!r} is not a valid {cls.__name__}")

    @property
    def __members__(cls):
        return dict(cls._member_map_)


# Legacy alias — CPython kept both names after renaming EnumMeta -> EnumType.
EnumMeta = EnumType


class Enum(metaclass=EnumType):
    def __repr__(self):
        return f"<{self.__class__.__name__}.{self._name_}: {self._value_!r}>"

    def __str__(self):
        return f"{self.__class__.__name__}.{self._name_}"

    @property
    def name(self):
        return self._name_

    @property
    def value(self):
        return self._value_

    @staticmethod
    def _generate_next_value_(name, start, count, last_values):
        return count


class IntEnum(int, Enum):
    pass


class StrEnum(str, Enum):
    # Deliberately does not override __str__/__repr__ to return the raw
    # string value (real CPython's StrEnum does, via `str.__str__(self)`) —
    # `str` here is a bare BuiltinFunction (see is_recognized_native_base_name
    # in object.rs), not a real class object, so its own `__str__` can't be
    # reached directly by name the way CPython does it. Equality/hashing/use
    # as an actual string (DB serialization, string concatenation, etc.)
    # still work correctly via the native str backing's normal delegation;
    # only `str(member)`'s cosmetic output differs (shows "ClassName.MEMBER"
    # like a plain Enum, instead of the raw value).
    @staticmethod
    def _generate_next_value_(name, start, count, last_values):
        return name.lower()


def unique(enumeration):
    # `_member_map_` includes ALIASES (same-valued members beyond the
    # first, which EnumType.__new__ already collapsed to point at the
    # canonical member instead of creating a separate one) — an alias's own
    # key never matches its target's real `_name_`, which is exactly what
    # identifies it as a duplicate here. Checking `_member_names_` instead
    # (as this used to) can never find anything: aliasing already happened
    # before `unique()` runs, so no two *canonical* members ever share a
    # value by construction.
    duplicates = [
        (name, member._name_)
        for name, member in enumeration._member_map_.items()
        if name != member._name_
    ]
    if duplicates:
        alias_details = ", ".join(f"{alias} -> {name}" for alias, name in duplicates)
        raise ValueError(f"duplicate values found in {enumeration!r}: {alias_details}")
    return enumeration
