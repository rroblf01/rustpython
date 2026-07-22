"""A real (not stub) implementation of the core of the standard
``dataclasses`` module, tailored to this interpreter.

Deliberately simplified relative to real CPython's ``dataclasses.py``: no
lazy/string-format annotation support (that needs a much more complete
``annotationlib``/``ast`` module than this interpreter currently has — see
the project's compat notes), no ``__slots__`` generation, no ``match_args``/
``kw_only`` defaults interacting with ``__future__`` annotations, no
``ClassVar``/``InitVar`` detection beyond a simple string/typing check. Covers
the overwhelming majority of real-world ``@dataclass`` usage: field
generation from annotations (in definition order), ``field()`` with
``default``/``default_factory``/``init``/``repr``/``compare``/``hash``/
``kw_only``/``metadata``, generated ``__init__``/``__repr__``/``__eq__``
(and ``__lt__``/etc. for ``order=True``, ``__hash__`` per the real
default/frozen/eq/unsafe_hash rules), ``__dataclass_fields__``, ``fields()``,
``is_dataclass()``, ``asdict()``/``astuple()`` (recursive, matching real
semantics for nested dataclasses/lists/tuples/dicts), ``replace()``, and
``make_dataclass()``.
"""

__all__ = [
    "dataclass", "field", "Field", "FrozenInstanceError",
    "fields", "asdict", "astuple", "is_dataclass", "replace",
    "make_dataclass", "MISSING", "KW_ONLY",
]


class _MissingType:
    def __repr__(self):
        return "MISSING"


MISSING = _MissingType()


class _KwOnlyType:
    def __repr__(self):
        return "KW_ONLY"


KW_ONLY = _KwOnlyType()


class FrozenInstanceError(AttributeError):
    pass


class Field:
    __slots__ = (
        "name", "type", "default", "default_factory", "init", "repr",
        "hash", "compare", "metadata", "kw_only",
    )

    def __init__(self, default, default_factory, init, repr, hash, compare,
                 metadata, kw_only):
        self.name = None
        self.type = None
        self.default = default
        self.default_factory = default_factory
        self.init = init
        self.repr = repr
        self.hash = hash
        self.compare = compare
        self.metadata = metadata if metadata is not None else {}
        self.kw_only = kw_only

    def __repr__(self):
        return (
            "Field(name=%r,type=%r,default=%r,default_factory=%r,"
            "init=%r,repr=%r,hash=%r,compare=%r,metadata=%r,kw_only=%r)"
            % (self.name, self.type, self.default, self.default_factory,
               self.init, self.repr, self.hash, self.compare,
               self.metadata, self.kw_only)
        )


def field(*, default=MISSING, default_factory=MISSING, init=True, repr=True,
          hash=None, compare=True, metadata=None, kw_only=MISSING):
    if default is not MISSING and default_factory is not MISSING:
        raise ValueError("cannot specify both default and default_factory")
    return Field(default, default_factory, init, repr, hash, compare,
                 metadata, kw_only)


def _is_classvar_or_initvar(annotation):
    if isinstance(annotation, str):
        return annotation.startswith("ClassVar") or annotation.startswith("InitVar")
    name = getattr(annotation, "__name__", None)
    if name in ("ClassVar", "InitVar"):
        return True
    origin = getattr(annotation, "__origin__", None)
    return getattr(origin, "__name__", None) == "ClassVar"


def _collect_fields(cls):
    """Fields from base classes (in mro order, base-to-derived so a
    subclass's own re-declaration overrides a base's), then this class's
    own annotations appended in definition order — matching real
    dataclasses' field-ordering rule."""
    fields_dict = {}
    for base in reversed(cls.__mro__[1:]):
        base_fields = base.__dict__.get("__dataclass_fields__")
        if base_fields:
            for name, f in base_fields.items():
                fields_dict[name] = f

    own_annotations = cls.__dict__.get("__annotations__", {})
    seen_kw_only = False
    for name, annotation in own_annotations.items():
        if annotation is KW_ONLY or (isinstance(annotation, str) and annotation == "KW_ONLY"):
            seen_kw_only = True
            continue
        if _is_classvar_or_initvar(annotation):
            continue
        raw_default = cls.__dict__.get(name, MISSING)
        if isinstance(raw_default, Field):
            f = raw_default
        else:
            f = Field(raw_default, MISSING, True, True, None, True, None, MISSING)
        f.name = name
        f.type = annotation
        if f.kw_only is MISSING:
            f.kw_only = seen_kw_only
        fields_dict[name] = f
        # A Field()/default value left as a plain class attribute would
        # otherwise be inherited by every instance directly (shadowing the
        # generated __init__'s own assignment) — remove it, same as real
        # dataclasses, UNLESS it's a plain, non-mutable-looking default
        # that real code may still want reachable as ClassName.attr... real
        # CPython actually does delete/reset these too, so match that.
        if name in cls.__dict__:
            if isinstance(raw_default, Field) or f.default is not MISSING:
                try:
                    delattr(cls, name)
                except AttributeError:
                    pass

    return fields_dict


def _make_init(cls, fields_list, frozen):
    std_fields = [f for f in fields_list if f.init and not f.kw_only]
    kw_fields = [f for f in fields_list if f.init and f.kw_only]

    globals_ns = {"MISSING": MISSING}
    params = ["self"]
    body = []

    def add_param(f):
        pname = f.name
        if f.default_factory is not MISSING:
            globals_ns["_dflt_factory_%s" % pname] = f.default_factory
            params.append("%s=MISSING" % pname)
        elif f.default is not MISSING:
            globals_ns["_dflt_%s" % pname] = f.default
            params.append("%s=_dflt_%s" % (pname, pname))
        else:
            params.append(pname)

    for f in std_fields:
        add_param(f)
    if kw_fields:
        params.append("*")
        for f in kw_fields:
            add_param(f)

    # A frozen dataclass overrides __setattr__ to reject any assignment to
    # a field — including the field-initializing assignments __init__
    # itself needs to make. Real dataclasses' generated __init__ bypasses
    # this the same way: call object.__setattr__ directly instead of plain
    # `self.name = value` syntax.
    def assign(name, value_expr):
        if frozen:
            return "    object.__setattr__(self, %r, %s)" % (name, value_expr)
        return "    self.%s = %s" % (name, value_expr)

    for f in fields_list:
        if not f.init:
            if f.default_factory is not MISSING:
                globals_ns["_dflt_factory_%s" % f.name] = f.default_factory
                body.append(assign(f.name, "_dflt_factory_%s()" % f.name))
            elif f.default is not MISSING:
                globals_ns["_dflt_%s" % f.name] = f.default
                body.append(assign(f.name, "_dflt_%s" % f.name))
            continue
        if f.default_factory is not MISSING:
            body.append(assign(
                f.name,
                "_dflt_factory_%s() if %s is MISSING else %s" % (f.name, f.name, f.name),
            ))
        else:
            body.append(assign(f.name, f.name))

    post_init_fields = [f.name for f in fields_list if not f.init]
    if hasattr(cls, "__post_init__"):
        init_only = [f.name for f in fields_list if f.init]
        body.append("    self.__post_init__()")

    if not body:
        body.append("    pass")

    src = "def __init__(%s):\n%s\n" % (", ".join(params), "\n".join(body))
    exec(src, globals_ns)
    return globals_ns["__init__"]


def _make_repr(fields_list):
    repr_fields = [f.name for f in fields_list if f.repr]

    def __repr__(self):
        parts = ", ".join(
            "%s=%r" % (name, getattr(self, name)) for name in repr_fields
        )
        return "%s(%s)" % (type(self).__name__, parts)

    return __repr__


def _make_eq(fields_list):
    cmp_fields = [f.name for f in fields_list if f.compare]

    def __eq__(self, other):
        if type(self) is not type(other):
            return NotImplemented
        return tuple(getattr(self, n) for n in cmp_fields) == tuple(getattr(other, n) for n in cmp_fields)

    return __eq__


def _make_cmp(name, op):
    def cmp(self, other, _op=op):
        if type(self) is not type(other):
            return NotImplemented
        sfields = self.__dataclass_fields__
        cmp_names = [f.name for f in sfields.values() if f.compare]
        st = tuple(getattr(self, n) for n in cmp_names)
        ot = tuple(getattr(other, n) for n in cmp_names)
        return _op(st, ot)

    cmp.__name__ = name
    return cmp


def _make_hash(fields_list):
    hash_fields = [f.name for f in fields_list if (f.hash if f.hash is not None else f.compare)]

    def __hash__(self):
        return hash(tuple(getattr(self, n) for n in hash_fields))

    return __hash__


def _frozen_setattr(self, name, value):
    if name in type(self).__dataclass_fields__:
        raise FrozenInstanceError("cannot assign to field %r" % name)
    object.__setattr__(self, name, value)


def _frozen_delattr(self, name):
    if name in type(self).__dataclass_fields__:
        raise FrozenInstanceError("cannot delete field %r" % name)
    object.__delattr__(self, name)


def _process_class(cls, init, repr, eq, order, unsafe_hash, frozen, kw_only):
    fields_dict = _collect_fields(cls)
    if kw_only:
        for f in fields_dict.values():
            if f.kw_only is MISSING:
                f.kw_only = True
    fields_list = list(fields_dict.values())

    cls.__dataclass_fields__ = fields_dict
    cls.__dataclass_params__ = {
        "init": init, "repr": repr, "eq": eq, "order": order,
        "unsafe_hash": unsafe_hash, "frozen": frozen,
    }

    if init and "__init__" not in cls.__dict__:
        cls.__init__ = _make_init(cls, fields_list, frozen)
    if repr and "__repr__" not in cls.__dict__:
        cls.__repr__ = _make_repr(fields_list)
    if eq and "__eq__" not in cls.__dict__:
        cls.__eq__ = _make_eq(fields_list)
    if order:
        import operator
        cls.__lt__ = _make_cmp("__lt__", operator.lt)
        cls.__le__ = _make_cmp("__le__", operator.le)
        cls.__gt__ = _make_cmp("__gt__", operator.gt)
        cls.__ge__ = _make_cmp("__ge__", operator.ge)

    if frozen:
        cls.__setattr__ = _frozen_setattr
        cls.__delattr__ = _frozen_delattr
        if eq and (unsafe_hash or "__hash__" not in cls.__dict__):
            cls.__hash__ = _make_hash(fields_list)
    elif unsafe_hash:
        cls.__hash__ = _make_hash(fields_list)
    elif eq and "__init__" not in cls.__dict__ and "__hash__" not in cls.__dict__:
        # eq=True (default) with no explicit __hash__ makes instances
        # unhashable, matching real dataclasses (mutable-by-default).
        cls.__hash__ = None

    return cls


def dataclass(cls=None, /, *, init=True, repr=True, eq=True, order=False,
              unsafe_hash=False, frozen=False, kw_only=False):
    def wrap(c):
        return _process_class(c, init, repr, eq, order, unsafe_hash, frozen, kw_only)

    if cls is None:
        return wrap
    return wrap(cls)


def fields(class_or_instance):
    cls = class_or_instance if isinstance(class_or_instance, type) else type(class_or_instance)
    try:
        d = cls.__dataclass_fields__
    except AttributeError:
        raise TypeError("%r is not a dataclass" % (class_or_instance,))
    return tuple(d.values())


def is_dataclass(obj):
    cls = obj if isinstance(obj, type) else type(obj)
    return hasattr(cls, "__dataclass_fields__")


def _asdict_inner(obj):
    if is_dataclass(obj):
        return {f.name: _asdict_inner(getattr(obj, f.name)) for f in fields(obj)}
    if isinstance(obj, (list, tuple)):
        return type(obj)(_asdict_inner(v) for v in obj)
    if isinstance(obj, dict):
        return {k: _asdict_inner(v) for k, v in obj.items()}
    import copy
    return copy.deepcopy(obj)


def asdict(obj):
    if not is_dataclass(obj) or isinstance(obj, type):
        raise TypeError("asdict() should be called on dataclass instances")
    return _asdict_inner(obj)


def _astuple_inner(obj):
    if is_dataclass(obj):
        return tuple(_astuple_inner(getattr(obj, f.name)) for f in fields(obj))
    if isinstance(obj, (list, tuple)):
        return type(obj)(_astuple_inner(v) for v in obj)
    if isinstance(obj, dict):
        return {k: _astuple_inner(v) for k, v in obj.items()}
    import copy
    return copy.deepcopy(obj)


def astuple(obj):
    if not is_dataclass(obj) or isinstance(obj, type):
        raise TypeError("astuple() should be called on dataclass instances")
    return _astuple_inner(obj)


def replace(obj, /, **changes):
    if not is_dataclass(obj) or isinstance(obj, type):
        raise TypeError("replace() should be called on dataclass instances")
    for f in fields(obj):
        if not f.init and f.name not in changes:
            continue
        if f.name not in changes:
            changes[f.name] = getattr(obj, f.name)
    return type(obj)(**changes)


def make_dataclass(cls_name, fields, *, bases=(), namespace=None, init=True,
                    repr=True, eq=True, order=False, unsafe_hash=False,
                    frozen=False, kw_only=False):
    namespace = dict(namespace) if namespace else {}
    annotations = {}
    for item in fields:
        if isinstance(item, str):
            name, tp, default = item, "typing.Any", MISSING
        elif len(item) == 2:
            name, tp = item
            default = MISSING
        else:
            name, tp, default = item
        annotations[name] = tp
        if default is not MISSING:
            namespace[name] = default
    namespace["__annotations__"] = annotations
    cls = type(cls_name, bases, namespace)
    return dataclass(cls, init=init, repr=repr, eq=eq, order=order,
                      unsafe_hash=unsafe_hash, frozen=frozen, kw_only=kw_only)
