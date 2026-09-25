"""Real `enum` module implementation (Enum/IntEnum/StrEnum/EnumType/auto/
unique/nonmember/Flag/IntFlag/FlagBoundary), loaded as Python source via
VirtualMachine::install_source_defined_stdlib — see that function's doc
comment for why. Deliberately scoped to what's needed for real-world use
(including Django's `Choices`/`IntegerChoices`/`TextChoices`, which build a
custom metaclass on top of `EnumType`): no `__prepare__`-based duplicate-name
detection (this interpreter's class namespace is already delivered here in
definition order — see Frame::name_order / PyDict::order — which is the
part CPython's `_EnumDict` needs `__prepare__` for; strict duplicate
rejection is not), and `Flag`'s repr/str/decomposition logic is a
pragmatic reimplementation rather than a byte-for-byte port of CPython's
own (considerably more elaborate) `Lib/enum.py` `Flag` class.

Deliberately NOT included: `EnumCheck`/`verify` (the `@verify(CONTINUOUS,
...)` class-decorator validation feature). An earlier attempt to add them
here — including via real CPython's own `@_simple_enum(StrEnum)` class
decorator AND via a plain `class EnumCheck(StrEnum): ...` statement paired
with `CONTINUOUS, NAMED_FLAGS, UNIQUE = EnumCheck` (tuple-unpacking
iteration) — caused this module's OWN bootstrap (this source runs via
`VirtualMachine::install_source_defined_stdlib` on every fresh
`VirtualMachine::new()`, before that function's own process-wide result
cache exists yet) to either hang forever or leave the entire `enum` module
empty, depending on the exact shape. Root cause: something in that
combination needs a disposable VM (the same "nested Python-level calls
from Rust builtin code" pattern documented elsewhere in this codebase,
e.g. `call_bound_method`), and constructing one DURING this module's own
first-ever bootstrap re-enters this same not-yet-finished bootstrap.
`install_collections_abc_alias` has an explicit reentrancy guard for
exactly this class of hazard; `install_source_defined_stdlib` itself does
not, and giving it one is a separate, more involved fix than adding
`verify`/`EnumCheck` warrants on its own — left for a future, dedicated
pass rather than worked around here with something fragile.
"""
import sys

# Forward-reference sentinel (mirrors real CPython's own `Lib/enum.py`,
# which does the same for the same reason): `EnumType.__call__` below
# references the global name `Flag` to detect composite-value lookups on a
# Flag subclass, but `Flag` itself is defined much later in this same
# module — by the time any real `SomeFlag(value)` call happens (well after
# the whole module has finished loading), the real class has replaced this
# `None` placeholder, but the name must still exist for that lookup to be
# valid syntax during the (very unlikely, but not impossible) case of an
# early call before this module finishes executing.
Flag = None


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


# Public alias — CPython 3.13 renamed the internal `_EnumDict` to the
# public `EnumDict` (real trigger: CPython's own `test_enum.py` does
# `from enum import ... EnumDict`).
EnumDict = _EnumDict



_native_repr = repr


def _find_member_type(cls):
    """CPython's `EnumType._get_mixins_` member type: the first base in
    the MRO that is a real data type (int/str/float/bytes/complex, or a
    user data type that brought its own `__new__`), else `object`."""
    for base in getattr(cls, '__mro__', ())[1:]:
        name = base.__name__
        if name in ('object', 'Enum', 'Flag', 'IntFlag', 'ReprEnum',
                    'EnumType', 'EnumMeta'):
            continue
        if name in ('int', 'str', 'float', 'complex', 'bytes'):
            return base
        # A user data type: anything that is not one of the enum classes
        # themselves and is not the enum machinery.
        # Skip anything that is part of the enum machinery itself but was
        # not caught by name (a user subclass of Enum, an intermediate mixin
        # like `ReprEnum`). Forward references (`Flag`/`IntFlag` are defined
        # further down this module) are resolved by name.
        try:
            if issubclass(base, Enum):
                continue
        except TypeError:
            pass
        return base
    return object


def _find_new_member(cls):
    """The constructor CPython stores as `_new_member_`: the first
    MIXED-IN data type's `__new__` (`int.__new__` for an IntEnum), or
    `object.__new__` when there is no data type."""
    for base in getattr(cls, '__mro__', ())[1:]:
        if base.__name__ in ('object', 'Enum', 'Flag', 'IntFlag', 'ReprEnum',
                             'IntEnum', 'StrEnum', 'EnumType', 'EnumMeta'):
            continue
        for d in (base.__dict__,):
            new = d.get('__new__')
            if new is not None:
                return new
    return object.__new__


def _find_value_repr(cls):
    """CPython's `EnumType._find_data_repr_`: the repr function of the
    first MIXED-IN data type in the MRO (so an IntEnum member reprs its
    value with `int.__repr__`), or None when the enum has no data type."""
    for base in getattr(cls, '__mro__', ())[1:]:
        name = base.__name__
        if name in ('object', 'Enum', 'Flag', 'IntFlag', 'ReprEnum',
                    'EnumType', 'EnumMeta'):
            continue
        # The native scalar mixins: their repr is the builtin slot. Checked
        # by name (not by "__new__ in __dict__") because this codebase's
        # native type dicts don't carry every slot.
        if name in ('int', 'str', 'float', 'complex', 'bytes'):
            repr_fn = getattr(base, '__repr__', None)
            return repr_fn if repr_fn is not None else _native_repr
        repr_fn = base.__dict__.get('__repr__')
        if repr_fn is not None:
            return repr_fn
    return None

class EnumType(type):
    @classmethod
    def __prepare__(metacls, name, bases, **kwds):
        return _EnumDict()

    def __new__(metacls, name, bases, namespace, **kwds):
        # _simple_enum support: CPython's EnumType.__new__ short-circuits
        # when _simple=True (called from _simple_enum's type(cls_name, (etype,), body, _simple=True))
        # to avoid the normal member-processing path; _simple_enum then populates
        # members manually.
        if kwds.pop("_simple", False):
            kwds.pop("boundary", None)
            return super().__new__(metacls, name, bases, namespace)
        # `class SomeFlags(Flag, boundary=CONFORM): ...` — stash the
        # requested boundary as a real (sunder, so never misclassified as
        # a member) class attribute instead of silently discarding it, so
        # `Flag.__new__`'s composite-value handling (added below) can find
        # it via normal attribute/MRO lookup on `cls._boundary_`.
        boundary = kwds.pop("boundary", None)
        if boundary is not None:
            namespace["_boundary_"] = boundary
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
        # `_member_type_` — the type of the members' values (CPython sets it
        # from the first member, falling back to the mixin). 766 of
        # test_enum's own assertions read it (`self.assertIs(MyEnum._member_type_,
        # int)` and the `_missing_`/`_value2member_map_` machinery).
        # `_member_type_` is the MIXED-IN data type (int/str/...),
        # defaulting to `object` — NOT the type of the first member's
        # value: CPython's `class C(Enum): A = 1` still reports
        # `_member_type_ is object` (it comes from `_get_mixins_`), and
        # `EnumType.__dir__` keys off exactly that to decide whether the
        # data type's own attributes belong in `dir()`.
        cls._member_type_ = _find_member_type(cls)
        # `_new_member_` / `_value_repr_` (CPython's EnumType.__new__ sets
        # both on every enum it builds). `_new_member_` is the constructor
        # used for members — `int.__new__` for an IntEnum, `object.__new__`
        # otherwise — and tests call it directly
        # (`Color._new_member_(Color, 3)`) and compare it against
        # `object.__new__` to decide whether `__new__` shows up in
        # `dir()`. `_value_repr_` is the MIXED-IN data type's repr function
        # (None for a plain Enum), which is what the member `__repr__`
        # formats the value with.
        cls._new_member_ = _find_new_member(cls)
        cls._value_repr_ = _find_value_repr(cls)
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

    def __call__(cls, value, *args, **kwargs):
        # Functional API: Color('NAME', 'A B', module=...). Detected by a
        # string name plus a names spec (str/list/tuple/dict) — real
        # trigger: plistlib's `enum.Enum('PlistFormat', ..., module=...)`.
        # The `names=`/`type=`/`start=` keyword forms count too: CPython's
        # `Enum('Empty', type=int)` builds a MEMBERLESS enum (test_enum's
        # `TestMixedIntFunction` does exactly that), and a lone string with
        # no names spec at all is a member lookup that CPython rejects with
        # "has no members; specify `names=()` if you meant to create a new,
        # empty, enum".
        if isinstance(value, str) and (
            'names' in kwargs or 'type' in kwargs or 'start' in kwargs
        ):
            names = kwargs.pop('names', None)
            if args:
                names = args[0]
                args = args[1:]
            return cls._create_functional(value, names, *args, **kwargs)
        if isinstance(value, str) and args and isinstance(
            args[0], (str, list, tuple, dict)
        ):
            return cls._create_functional(value, args[0], *args[1:], **kwargs)
        if not args and not kwargs and isinstance(value, cls):
            return value
        try:
            return cls._value2member_map_[value]
        except (KeyError, TypeError):
            for m in cls:
                if m._value_ == value:
                    return m
            # Flag/IntFlag: a value not matching any single canonical
            # member may still be a valid COMBINATION of them (e.g.
            # `Color(Color.RED | Color.BLUE)`) — `Flag` is defined further
            # down this same module and is a plain global by the time any
            # real `SomeFlag(value)` call happens (well after module load),
            # so this simple name lookup is safe despite the forward
            # reference. `Flag.__new__` (not this `__call__`) does the
            # actual decomposition/boundary handling.
            if Flag is not None and issubclass(cls, Flag):
                return cls.__new__(cls, value)
            raise ValueError(f"{value!r} is not a valid {cls.__name__}")

    def _create_functional(cls, name, names, *args, module=None, qualname=None, type=None, **kwargs):
        # Parse names spec like CPython's _create_.
        if names is None:
            names = ()
        if isinstance(names, str):
            names = names.split()
        if isinstance(names, (list, tuple)):
            seq = list(names)
            # A sequence of (name, value) PAIRS keeps its values
            # (`Flag('Perm', (('R', 2), ('W', 8)))` — test_enum's
            # `test_programatic_function_iterable`); a bare sequence of
            # names is auto-numbered from 1.
            if seq and all(
                isinstance(n, (tuple, list)) and len(n) == 2 and isinstance(n[0], str)
                for n in seq
            ):
                items = [(n[0], n[1]) for n in seq]
            else:
                items = [(n, i + 1) for i, n in enumerate(seq)]
        elif hasattr(names, 'items'):
            items = list(names.items())
        else:
            # Any other iterable of pairs (`itertools`-style specs).
            seq = list(names)
            items = (
                [(n[0], n[1]) for n in seq]
                if all(
                    isinstance(n, (tuple, list)) and len(n) == 2 for n in seq
                )
                else [(n, i + 1) for i, n in enumerate(seq)]
            )
        ns = _EnumDict()
        ns["__module__"] = module if module is not None else cls.__module__
        if qualname is not None:
            ns["__qualname__"] = qualname
        for k, v in items:
            ns[k] = v
        bases = (cls,) if type is None else (type, cls)
        # Direct metaclass construction (avoids re-entering __call__).
        return EnumType.__new__(EnumType, name, bases, ns)

    @classmethod
    def _convert_(cls, name, module, filter, source=None, *,
                  boundary=None, as_global=False):
        """Create a new Enum subclass from a collection of constants.

        CPython's `EnumType._convert_` (3.13+): every name in `source`
        (a module/namespace, defaulting to `module`'s globals) that
        `filter(name)` accepts becomes a member of a freshly built enum
        called `name`, which is then exported back into the module.
        `_value2member_map_` is filled in sorted-by-value order so the
        reverse lookup is stable when several names share a value.
        """
        module_globals = sys.modules[module].__dict__
        if source:
            source = source.__dict__
        else:
            source = module_globals
        members = [
            (key, value)
            for key, value in source.items()
            if filter(key)
        ]
        try:
            members.sort(key=lambda t: (t[1], t[0]))
        except TypeError:
            # values that aren't comparable with each other: sort by name
            members.sort(key=lambda t: t[0])
        body = {t[0]: t[1] for t in members}
        body['__module__'] = module
        tmp_cls = type(name, (object, ), body)
        new_cls = _simple_enum(etype=cls, boundary=boundary or KEEP)(tmp_cls)
        if as_global:
            global_enum(new_cls)
        else:
            sys.modules[new_cls.__module__].__dict__.update(new_cls.__members__)
        module_globals[name] = new_cls
        return new_cls

    def __dir__(cls):
        """Customize `dir()` to hide the enum's internal bookkeeping.

        CPython filters out `_member_names_`/`_member_map_`/`_new_member_`/
        `_value_repr_`/`_generate_next_value_` and friends, which would
        otherwise sit right next to the real members and confuse every
        introspection tool (and test_enum's own `enum_dir`, which asserts
        `set(dir(SomeEnum))` matches exactly this list).
        """
        interesting = set([
            '__class__', '__contains__', '__doc__', '__getitem__',
            '__iter__', '__len__', '__members__', '__module__',
            '__name__', '__qualname__',
        ] + list(getattr(cls, '_member_names_', ())))
        if getattr(cls, '_new_member_', None) is not object.__new__:
            interesting.add('__new__')
        if getattr(cls, '__init_subclass__', None) is not object.__init_subclass__:
            interesting.add('__init_subclass__')
        member_type = getattr(cls, '_member_type_', object)
        if member_type is object:
            return sorted(interesting)
        return sorted(set(dir(member_type)) | interesting)

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


class ReprEnum(Enum):
    """Only changes the repr(), leaving str() and format() to the mixed-in
    type. Real CPython's `IntEnum`/`StrEnum` actually subclass this rather
    than `Enum` directly — kept as a standalone class here (not spliced
    into `IntEnum`/`StrEnum`'s own bases below) to avoid touching their
    already-working, separately-tuned behavior; this exists so code that
    imports/references `enum.ReprEnum` directly (real trigger: CPython's
    own `test_enum.py`) finds a real class rather than an ImportError."""


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


class FlagBoundary(StrEnum):
    """How to handle out-of-range/extra bits in a Flag value — matches
    real CPython's `enum.FlagBoundary` member names/semantics."""

    STRICT = "strict"
    CONFORM = "conform"
    EJECT = "eject"
    KEEP = "keep"


STRICT = FlagBoundary.STRICT
CONFORM = FlagBoundary.CONFORM
EJECT = FlagBoundary.EJECT
KEEP = FlagBoundary.KEEP


def _high_bit(value):
    return value.bit_length() - 1


def _is_single_bit(num):
    """True if only one bit is set in num (should be an int)."""
    if num == 0:
        return False
    num &= num - 1
    return num == 0


def _iter_bits_lsb(num):
    # num must be a positive integer (or a Flag/IntFlag member, whose
    # `.value` is used).
    original = num
    if isinstance(num, Flag):
        num = num._value_
    if num < 0:
        raise ValueError(f"{original!r} is not a positive integer")
    while num:
        b = num & (~num + 1)
        yield b
        num ^= b


def _decompose_flag_bits(cls, value):
    """Split `value` into the canonical (already-defined) single/multi-bit
    members it's covered by, plus whatever bits remain unaccounted for.
    Real CPython's `_decompose` is considerably more elaborate (handles
    single-bit-only decomposition preference, pseudo-member caching
    details, etc.) — this is a pragmatic version sufficient for ordinary
    Flag usage (membership tests, iteration, repr)."""
    members = []
    remaining = value
    for m in cls._member_map_.values():
        mv = m._value_
        if mv and (mv & value) == mv and (remaining & mv):
            members.append(m)
            remaining &= ~mv
    return members, remaining


class Flag(Enum):
    """Members combine via `|`/`&`/`^`/`~`; a value that isn't an exact
    canonical member is decomposed into the canonical members whose bits
    it covers (or rejected/masked/passed-through/kept, per `_boundary_`)."""

    _boundary_ = STRICT

    @staticmethod
    def _generate_next_value_(name, start, count, last_values):
        # Real CPython: `auto()` inside a Flag picks the next power of two
        # after the highest bit used so far (1, 2, 4, 8, ...), not a plain
        # sequential count — members are meant to be independently
        # combinable bits.
        if not count:
            return 1
        for last_value in reversed(last_values):
            try:
                high_bit = _high_bit(last_value)
                break
            except (AttributeError, TypeError):
                raise TypeError(f"Invalid Flag value: {last_value!r}") from None
        else:
            return 1
        return 2 ** (high_bit + 1)

    def __new__(cls, value):
        # Looking up (or building a pseudo-member for) a possibly-composite
        # value — real member creation during class-body processing always
        # goes through `object.__new__` directly (see `EnumType.__new__`
        # above), never through here.
        if isinstance(value, cls):
            return value
        if not isinstance(value, int):
            raise ValueError(f"{value!r} is not a valid {cls.__name__}")
        try:
            return cls._value2member_map_[value]
        except (KeyError, TypeError):
            pass
        members, extra = _decompose_flag_bits(cls, value)
        boundary = getattr(cls, "_boundary_", STRICT)
        if extra:
            if boundary is STRICT:
                raise ValueError(
                    f"{value!r} is not a valid {cls.__name__}"
                )
            elif boundary is CONFORM:
                value = value & ~extra
                extra = 0
            elif boundary is EJECT:
                return value
            # KEEP: fall through, keeping the extra bits in the pseudo-member
        # Always pass `value` through to `object.__new__` — same convention
        # `EnumType.__new__` uses for regular member creation above; the
        # Rust side ignores the extra arg when `cls` doesn't transparently
        # subclass a native type.
        pseudo = object.__new__(cls, value)
        name_parts = [m._name_ for m in members]
        if extra:
            # Real CPython's own KEEP-boundary repr/str includes the
            # leftover, not-covered-by-any-member bits as a plain decimal
            # suffix (e.g. `Loose.RED|8`), not just the canonical member
            # names — folding it into `_name_` here (rather than only in
            # `__repr__`/`__str__` below) keeps both consistent for free.
            name_parts.append(str(extra))
        pseudo._name_ = "|".join(name_parts) if name_parts else None
        pseudo._value_ = value
        cls._value2member_map_[value] = pseudo
        return pseudo

    def __or__(self, other):
        cls = self.__class__
        other_val = other._value_ if isinstance(other, cls) else other
        return cls(self._value_ | other_val)

    def __and__(self, other):
        cls = self.__class__
        other_val = other._value_ if isinstance(other, cls) else other
        return cls(self._value_ & other_val)

    def __xor__(self, other):
        cls = self.__class__
        other_val = other._value_ if isinstance(other, cls) else other
        return cls(self._value_ ^ other_val)

    __ror__ = __or__
    __rand__ = __and__
    __rxor__ = __xor__

    def __invert__(self):
        cls = self.__class__
        all_bits = 0
        for m in cls._member_map_.values():
            all_bits |= m._value_
        return cls(all_bits ^ self._value_)

    def __bool__(self):
        return bool(self._value_)

    def __contains__(self, other):
        cls = self.__class__
        if not isinstance(other, cls):
            raise TypeError(
                f"unsupported operand type(s) for 'in': '{other.__class__.__name__}' and '{cls.__name__}'"
            )
        if other._value_ == 0:
            return self._value_ == 0
        return (other._value_ & self._value_) == other._value_

    def __iter__(self):
        members, _ = _decompose_flag_bits(self.__class__, self._value_)
        return iter(members)

    def __len__(self):
        members, _ = _decompose_flag_bits(self.__class__, self._value_)
        return len(members)

    def __repr__(self):
        cls_name = self.__class__.__name__
        # `_name_` is set (`Flag.__new__` above) for every value, whether a
        # single canonical member, a composite of several ("RED|BLUE"), or
        # one with leftover KEEP-boundary bits ("RED|8") — `None` only for
        # the zero/no-bits-matched value.
        label = self._name_ if self._name_ is not None else "0"
        return f"<{cls_name}.{label}: {self._value_!r}>"

    def __str__(self):
        cls_name = self.__class__.__name__
        label = self._name_ if self._name_ is not None else str(self._value_)
        return f"{cls_name}.{label}"


class IntFlag(int, Flag):
    """Members are also real ints (bitwise ops on plain ints against an
    IntFlag member work directly via the native int backing)."""

    _boundary_ = KEEP


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


def _is_dunder(name):
    return (
        len(name) > 4
        and name[:2] == name[-2:] == "__"
        and name[2] != "_"
        and name[-3] != "_"
    )


def _is_sunder(name):
    return (
        len(name) > 2
        and name[0] == name[-1] == "_"
        and name[1] != "_"
        and name[-2] != "_"
    )


def _is_private(cls_name, name):
    pattern = f"_{cls_name}__"
    pat_len = len(pattern)
    if (
        len(name) > pat_len
        and name.startswith(pattern)
        and (name[-1] != "_" or name[-2] != "_")
    ):
        return True
    return False


def _simple_enum(etype=Enum, *, boundary=None, use_args=None):
    """Class decorator that converts a plain class into an Enum.
    Simplified port of CPython's Lib/enum.py _simple_enum, sufficient for
    pstats.SortKey, http.HTTPStatus/HTTPMethod, uuid.SafeUUID and
    _ast_unparse._Precedence (auto, aliases, tuple values and custom __new__).
    """
    def decorator(cls):
        nonlocal use_args
        cls_name = cls.__name__
        # Determine use_args default like CPython: etype._use_args_
        if use_args is None:
            try:
                use_args = etype._use_args_
            except AttributeError:
                use_args = False
            # If the decorated class defines its own __new__, it almost
            # certainly expects the tuple values to be unpacked (e.g.
            # HTTPStatus: (value, phrase, description) -> __new__(cls, value, phrase, description))
            if "__new__" in cls.__dict__:
                use_args = True

        # Resolve the member creation function
        __new__ = cls.__dict__.get("__new__")
        new_member = None
        if __new__ is not None:
            # Unwrap staticmethod/function
            if isinstance(__new__, staticmethod):
                new_member = __new__.__func__
            elif hasattr(__new__, "__func__"):
                try:
                    new_member = __new__.__func__
                except AttributeError:
                    new_member = __new__
            else:
                new_member = __new__
        else:
            member_type = getattr(etype, "_member_type_", None)
            if member_type is None:
                # Infer from etype's MRO (IntEnum -> int, StrEnum -> str)
                try:
                    if issubclass(etype, int) and etype is not int:
                        member_type = int
                    elif issubclass(etype, str):
                        member_type = str
                    else:
                        member_type = object
                except Exception:
                    member_type = object
            if member_type is not object:
                new_member = member_type.__new__
            else:
                new_member = object.__new__
        # Re-evaluate member_type for later checks
        member_type = getattr(etype, "_member_type_", None)
        if member_type is None:
            try:
                if issubclass(etype, int) and etype is not int:
                    member_type = int
                elif issubclass(etype, str):
                    member_type = str
                else:
                    member_type = object
            except Exception:
                member_type = object

        attrs = {}
        body = {}
        if __new__ is not None:
            body["__new_member__"] = new_member
        body["_new_member_"] = new_member
        body["_use_args_"] = use_args
        # _generate_next_value_
        try:
            body["_generate_next_value_"] = gnv = etype._generate_next_value_
        except AttributeError:
            body["_generate_next_value_"] = gnv = _generate_next_value
        body["_member_names_"] = member_names = []
        body["_member_map_"] = member_map = {}
        body["_value2member_map_"] = value2member_map = {}
        body["_member_type_"] = member_type
        # Separate attrs vs body similar to CPython
        # Also exclude any name starting with "_" (e.g. _abc_registry, _abc_impl added by RustPython's type machinery)
        # — CPython's plain class wouldn't have these, but our interpreter injects them.
        for name, obj in cls.__dict__.items():
            if name in ("__dict__", "__weakref__"):
                continue
            if name.startswith("_") or _is_dunder(name) or _is_private(cls_name, name) or _is_sunder(name) or _is_descriptor(obj):
                body[name] = obj
            else:
                attrs[name] = obj
        if cls.__dict__.get("__doc__") is None:
            body["__doc__"] = "An enumeration."

        # Create enum class without triggering normal EnumType member processing
        try:
            enum_class = EnumType(cls_name, (etype,), body, _simple=True)
        except TypeError:
            enum_class = type.__new__(EnumType, cls_name, (etype,), body)

        # Ensure builtins for repr handling minimal (skip CPython's mixin __repr__ fixup)

        gnv_last_values = []
        for name, orig_value in list(attrs.items()):
            value = orig_value
            # handle auto() — mutate the shared auto instance so alias
            # `BOR = EXPR` (where both names point at the same auto object)
            # resolves to the same value
            if isinstance(value, auto):
                if value.value is None:
                    resolved = gnv(name, 1, len(member_names) + 1, list(gnv_last_values))
                    value.value = resolved
                    value = resolved
                else:
                    value = value.value

            if use_args:
                if not isinstance(value, tuple):
                    value = (value,)
                try:
                    member = new_member(enum_class, *value)
                except Exception:
                    member = object.__new__(enum_class)
                    # fallback set value
                # Determine canonical value for mapping (first element)
                raw_value = value[0] if len(value) == 1 else value[0] if len(value) > 0 else None
                # CPython uses value = value[0] after member creation when use_args
                # For mapping we use member._value_ if set, else raw_value
                check_value = getattr(member, "_value_", raw_value)
                # If custom __new__ didn't set _value_, set it if __new__ is None case handled below
                if not hasattr(member, "_value_"):
                    if __new__ is None:
                        member._value_ = raw_value
                        check_value = raw_value
                    else:
                        # custom __new__ should have set, but if not, use raw_value
                        try:
                            member._value_ = raw_value
                            check_value = raw_value
                        except Exception:
                            pass
                # For use_args False path, value is raw_value now
                value = raw_value
            else:
                # Non-use_args: need to create member with native backing if needed
                if member_type is not object:
                    try:
                        member = new_member(enum_class, value)
                    except Exception:
                        member = object.__new__(enum_class)
                        member._value_ = value
                    # `int.__new__`/`str.__new__` only build the native
                    # backing — the enum's own `_value_` is this
                    # implementation's bookkeeping and has to be set here
                    # even on the success path (without it every member of an
                    # `_simple_enum`-converted IntEnum/StrEnum lost `.value`).
                    if not hasattr(member, "_value_"):
                        try:
                            member._value_ = value
                        except Exception:
                            pass
                else:
                    member = new_member(enum_class)
                    member._value_ = value
                check_value = getattr(member, "_value_", value)

            # Alias detection via value2member_map
            try:
                contained = value2member_map.get(check_value)
            except TypeError:
                contained = None
                for m in enum_class:
                    if getattr(m, "_value_", None) == check_value or getattr(m, "value", None) == check_value:
                        contained = m
                        break
            if contained is not None:
                member_map[name] = contained
                setattr(enum_class, name, contained)
                # For multi-value alias like SortKey('calls','ncalls'), the alias 'ncalls' is handled via custom __new__ already adding to value2member_map
                # But for simple alias like BOR = EXPR, we already alias here
                continue
            else:
                member._name_ = name
                try:
                    member.__objclass__ = enum_class
                except Exception:
                    pass
                try:
                    member.__init__(value)
                except Exception:
                    pass
                member._sort_order_ = len(member_names)
                if name not in ("name", "value"):
                    setattr(enum_class, name, member)
                    member_map[name] = member
                else:
                    setattr(enum_class, name, member)
                    member_map[name] = member
                member_names.append(name)
                gnv_last_values.append(value)
                try:
                    value2member_map.setdefault(check_value, member)
                except TypeError:
                    pass
                # If member has _all_values (SortKey multi-alias), ensure mapping for extra values already done by custom __new__
                # custom __new__ for SortKey iterates values[1:] and does cls._value2member_map_[other]=obj
                # That dict is same object as value2member_map, so already handled

        if "__new__" in body:
            try:
                enum_class.__new_member__ = enum_class.__new__
            except Exception:
                pass
            enum_class.__new__ = Enum.__new__

        return enum_class
    return decorator


def _test_simple_enum(checked_enum, simple_enum):
    """Minimal stub of CPython's _test_simple_enum: compare member sets.
    Raises TypeError if differences are found, otherwise returns None.
    """
    try:
        ce = checked_enum
        se = simple_enum
        if set(ce._member_map_.keys()) != set(se._member_map_.keys()):
            raise TypeError(f"member keys differ: {set(ce._member_map_.keys()) ^ set(se._member_map_.keys())}")
        for k in ce._member_map_:
            ce_v = ce._member_map_[k]._value_
            se_v = se._member_map_[k]._value_
            if ce_v != se_v:
                raise TypeError(f"value mismatch for {k!r}: {ce_v!r} != {se_v!r}")
    except AttributeError as e:
        raise TypeError(str(e))
    return None


def _global_enum_repr(self):
    module = self.__class__.__module__.split('.')[-1]
    return '%s.%s' % (module, self._name_)


def _global_enum_str(self):
    module = self.__class__.__module__.split('.')[-1]
    return '%s.%s' % (module, self._name_)


def global_enum(cls, update_str=False):
    """Decorator: module-qualified repr + export members to module globals.

    No Flag/ReprEnum here, so always install both (CPython skips __str__
    for ReprEnum unless update_str).
    """
    cls.__repr__ = _global_enum_repr
    cls.__str__ = _global_enum_str
    # NB: must use setattr (not __dict__.update): module __dict__ writes
    # don't sync to frame globals (LOAD_GLOBAL), so exported members
    # would be invisible to module code (real trigger: calendar.MONDAY).
    try:
        mod = sys.modules[cls.__module__]
        for k, v in cls.__members__.items():
            setattr(mod, k, v)
    except Exception:
        pass


# ── `verify` / `EnumCheck` (the `@verify(CONTINUOUS)` class decorator) ──
# The module docstring above explains why this is NOT CPython's own
# `class EnumCheck(StrEnum)` + `CONTINUOUS, NAMED_FLAGS, UNIQUE =
# EnumCheck` (that shape needs a disposable VM while this very module is
# still bootstrapping). The constants only ever participate in `is`
# comparisons inside `verify.__call__` and are printed by nothing else, so a
# plain `str` subclass carries the same observable behavior (str value,
# equality with the string, repr) without touching the Enum machinery.
class EnumCheck(str):
    __str__ = str.__str__

    def __new__(cls, value):
        return str.__new__(cls, value)

    def __repr__(self):
        return f'<EnumCheck.{str(self).upper()}: {str(self)!r}>'


CONTINUOUS = EnumCheck('continuous')
NAMED_FLAGS = EnumCheck('named_flags')
UNIQUE = EnumCheck('unique')


class verify:
    """
    Check an enumeration for various constraints. (see EnumCheck)
    """
    def __init__(self, *checks):
        self.checks = checks
    def __call__(self, enumeration):
        checks = self.checks
        cls_name = enumeration.__name__
        if Flag is not None and issubclass(enumeration, Flag):
            enum_type = 'flag'
        elif issubclass(enumeration, Enum):
            enum_type = 'enum'
        else:
            raise TypeError("the 'verify' decorator only works with Enum and Flag")
        for check in checks:
            if check is UNIQUE:
                # check for duplicate names
                duplicates = []
                for name, member in enumeration.__members__.items():
                    if name != member.name:
                        duplicates.append((name, member.name))
                if duplicates:
                    alias_details = ', '.join(
                            ["%s -> %s" % (alias, name) for (alias, name) in duplicates])
                    raise ValueError('aliases found in %r: %s' %
                            (enumeration, alias_details))
            elif check is CONTINUOUS:
                values = set(e.value for e in enumeration)
                if len(values) < 2:
                    continue
                low, high = min(values), max(values)
                missing = []
                if enum_type == 'flag':
                    # check for powers of two
                    for i in range(_high_bit(low)+1, _high_bit(high)):
                        if 2**i not in values:
                            missing.append(2**i)
                elif enum_type == 'enum':
                    # check for missing consecutive integers
                    for i in range(low+1, high):
                        if i not in values:
                            missing.append(i)
                else:
                    raise Exception('verify: unknown type %r' % enum_type)
                if missing:
                    raise ValueError(('invalid %s %r: missing values %s' % (
                            enum_type, cls_name, ', '.join((str(m) for m in missing)))
                            )[:256])
                            # limit max length to protect against DOS attacks
            elif check is NAMED_FLAGS:
                # examine each alias and check for unnamed flags
                member_names = enumeration._member_names_
                member_values = [m.value for m in enumeration]
                missing_names = []
                missing_value = 0
                for name, alias in enumeration._member_map_.items():
                    if name in member_names:
                        # not an alias
                        continue
                    if alias.value < 0:
                        # negative numbers are not checked
                        continue
                    values = list(_iter_bits_lsb(alias.value))
                    missed = [v for v in values if v not in member_values]
                    if missed:
                        missing_names.append(name)
                        for val in missed:
                            missing_value |= val
                if missing_names:
                    if len(missing_names) == 1:
                        alias = 'alias %s is missing' % missing_names[0]
                    else:
                        alias = 'aliases %s and %s are missing' % (
                                ', '.join(missing_names[:-1]), missing_names[-1]
                                )
                    if _is_single_bit(missing_value):
                        value = 'value 0x%x' % missing_value
                    else:
                        value = 'combined values of 0x%x' % missing_value
                    raise ValueError(
                            'invalid Flag %r: %s %s [use enum.show_flag_values(value) for details]'
                            % (cls_name, alias, value)
                            )
        return enumeration
