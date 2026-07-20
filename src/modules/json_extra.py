"""Real `json.JSONEncoder` — loaded as Python source via
VirtualMachine::install_source_defined_stdlib (see that function's doc
comment for why), merged into the native `json` module which already
provides the fast-path `dumps`/`loads` for the common case.

Scoped to what real-world code needs from `JSONEncoder`: subclassing it and
overriding `default()` to teach it about extra types (Django's
`DjangoJSONEncoder`, extending it for datetime/Decimal/UUID, is the
motivating case) and passing an instance or a `cls=` to `json.dumps`. Not a
full reimplementation of CPython's streaming `iterencode` — `encode()`
walks the value once, replacing anything the native encoder can't handle
with `self.default(value)`'s result (recursively normalized the same way),
then hands the now-fully-native structure to the fast native `dumps`.
"""

import json as _json


class JSONEncoder:
    def __init__(
        self,
        *,
        skipkeys=False,
        ensure_ascii=True,
        check_circular=True,
        allow_nan=True,
        sort_keys=False,
        indent=None,
        separators=None,
        default=None,
        **kwargs,
    ):
        self.skipkeys = skipkeys
        self.ensure_ascii = ensure_ascii
        self.check_circular = check_circular
        self.allow_nan = allow_nan
        self.sort_keys = sort_keys
        self.indent = indent
        self.separators = separators
        if default is not None:
            self.default = default

    def default(self, o):
        raise TypeError(
            f"Object of type {o.__class__.__name__} is not JSON serializable"
        )

    def _normalize(self, o):
        if o is None or isinstance(o, (bool, int, float, str)):
            return o
        if isinstance(o, dict):
            return {k: self._normalize(v) for k, v in o.items()}
        if isinstance(o, (list, tuple)):
            return [self._normalize(v) for v in o]
        return self._normalize(self.default(o))

    def encode(self, o):
        normalized = self._normalize(o)
        return _json.dumps(
            normalized,
            self.indent if self.indent is not None else -1,
            self.sort_keys,
        )

    def iterencode(self, o, _one_shot=False):
        yield self.encode(o)


# Replaces the native module-level `dumps` (kept accessible as `_dumps`) with
# one that understands `cls=`/`default=` — the native fast path (plain
# dict/list/str/int/float/bool/None, the overwhelming common case) still
# goes straight through it; only when the caller actually wants custom
# serialization does this drop into `JSONEncoder` above.
_dumps = _json.dumps


def dumps(
    obj,
    *,
    skipkeys=False,
    ensure_ascii=True,
    check_circular=True,
    allow_nan=True,
    cls=None,
    indent=None,
    separators=None,
    default=None,
    sort_keys=False,
    **kw,
):
    if cls is None and default is None:
        return _dumps(obj, indent if indent is not None else -1, sort_keys)
    encoder_cls = cls if cls is not None else JSONEncoder
    return encoder_cls(
        skipkeys=skipkeys,
        ensure_ascii=ensure_ascii,
        check_circular=check_circular,
        allow_nan=allow_nan,
        sort_keys=sort_keys,
        indent=indent,
        separators=separators,
        default=default,
        **kw,
    ).encode(obj)
