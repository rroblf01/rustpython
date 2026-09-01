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
        # _simple_enum support: CPython's EnumType.__new__ short-circuits
        # when _simple=True (called from _simple_enum's type(cls_name, (etype,), body, _simple=True))
        # to avoid the normal member-processing path; _simple_enum then populates
        # members manually.
        if kwds.pop("_simple", False):
            kwds.pop("boundary", None)
            return super().__new__(metacls, name, bases, namespace)
        kwds.pop("boundary", None)
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
