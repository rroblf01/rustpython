// Split out of the former monolithic object.rs (see the file-splitting
// refactor's memory entry for context) — this file holds descriptor
// types (property/staticmethod/classmethod protocol support) and
// `__slots__` helpers.
use super::*;

// ---- Descriptor types ----

pub fn builtin_property(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let getter = if args.len() > 0 {
        Some(args[0].clone())
    } else {
        None
    };
    let setter = if args.len() > 1 {
        Some(args[1].clone())
    } else {
        None
    };
    let deleter = if args.len() > 2 {
        Some(args[2].clone())
    } else {
        None
    };
    let doc = if args.len() > 3 {
        Some(args[3].str())
    } else {
        // CPython property falls back to getter's __doc__
        getter.as_ref().and_then(|g| {
            let d = g.borrow().get_attribute("__doc__").ok()?;
            if matches!(&*d.borrow(), PyObject::None) { None } else { Some(d.str()) }
        })
    };
    Ok(PyObjectRef::new(PyObject::Property(Box::new(
        PropertyData {
            getter,
            setter,
            deleter,
            doc,
        },
    ))))
}

/// Return a new Property with the given getter (used by @x.getter)
pub fn property_getter(prop: &PyObjectRef, new_getter: PyObjectRef) -> PyObjectRef {
    let (setter, deleter, doc) = {
        let b = prop.borrow();
        match &*b {
            PyObject::Property(ref d) => (d.setter.clone(), d.deleter.clone(), d.doc.clone()),
            _ => return prop.clone(),
        }
    };
    PyObjectRef::new(PyObject::Property(Box::new(PropertyData {
        getter: Some(new_getter),
        setter,
        deleter,
        doc,
    })))
}

/// Builtin for property.getter(func) — returns new Property with getter
pub fn builtin_property_getter_fn(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 2 {
        return Err(PyError::type_error(
            "getter() requires at least the getter function",
        ));
    }
    Ok(property_getter(&args[0], args[1].clone()))
}

/// Return a new Property with the given setter (used by @x.setter)
pub fn property_setter(prop: &PyObjectRef, new_setter: PyObjectRef) -> PyObjectRef {
    let (getter, deleter, doc) = {
        let b = prop.borrow();
        match &*b {
            PyObject::Property(ref d) => (d.getter.clone(), d.deleter.clone(), d.doc.clone()),
            _ => return prop.clone(),
        }
    };
    PyObjectRef::new(PyObject::Property(Box::new(PropertyData {
        getter,
        setter: Some(new_setter),
        deleter,
        doc,
    })))
}

/// Return a new Property with the given deleter (used by @x.deleter)
pub fn property_deleter(prop: &PyObjectRef, new_deleter: PyObjectRef) -> PyObjectRef {
    let (getter, setter, doc) = {
        let b = prop.borrow();
        match &*b {
            PyObject::Property(ref d) => (d.getter.clone(), d.setter.clone(), d.doc.clone()),
            _ => return prop.clone(),
        }
    };
    PyObjectRef::new(PyObject::Property(Box::new(PropertyData {
        getter,
        setter,
        deleter: Some(new_deleter),
        doc,
    })))
}

/// Builtin for property.setter(func) — returns new Property with setter
pub fn builtin_property_setter_fn(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 2 {
        return Err(PyError::type_error(
            "setter() requires at least the setter function",
        ));
    }
    Ok(property_setter(&args[0], args[1].clone()))
}

/// Builtin for property.deleter(func) — returns new Property with deleter
pub fn builtin_property_deleter_fn(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 2 {
        return Err(PyError::type_error(
            "deleter() requires at least the deleter function",
        ));
    }
    Ok(property_deleter(&args[0], args[1].clone()))
}

pub fn builtin_staticmethod(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error(
            "staticmethod() requires at least 1 argument",
        ));
    }
    Ok(PyObjectRef::new(PyObject::StaticMethod {
        func: args[0].clone(),
    }))
}

pub fn builtin_classmethod(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error(
            "classmethod() requires at least 1 argument",
        ));
    }
    Ok(PyObjectRef::new(PyObject::ClassMethod {
        func: args[0].clone(),
    }))
}

// ---- __slots__ helpers ----

/// Extract slot names from a __slots__ value (can be str, tuple, list, or set)
pub(crate) fn extract_slots(slots_val: &PyObjectRef, result: &mut Vec<String>) {
    let borrowed = slots_val.borrow();
    match &*borrowed {
        PyObject::Str(s) => {
            if !result.iter().any(|x| x.as_str() == s.as_str()) {
                result.push(s.to_string());
            }
        }
        PyObject::Tuple(items) => {
            for item in items {
                if let PyObject::Str(s) = &*item.borrow() {
                    if !result.iter().any(|x| x.as_str() == s.as_str()) {
                        result.push(s.to_string());
                    }
                }
            }
        }
        PyObject::List(items) => {
            for item in items {
                if let PyObject::Str(s) = &*item.borrow() {
                    if !result.iter().any(|x| x.as_str() == s.as_str()) {
                        result.push(s.to_string());
                    }
                }
            }
        }
        PyObject::Set(set) => {
            for item in set.to_vec() {
                if let PyObject::Str(s) = &*item.borrow() {
                    if !result.iter().any(|x| x.as_str() == s.as_str()) {
                        result.push(s.to_string());
                    }
                }
            }
        }
        _ => {}
    }
}

/// Resolve `str.find`/`index`/`count`/etc.'s optional `start`/`end`
/// arguments into clamped, in-bounds `[start, end)` CHARACTER indices —
/// same semantics as slice bounds (negative indices count from the end,
/// out-of-range values clamp rather than error).
pub(crate) fn resolve_str_slice_bounds(
    len: usize,
    start: Option<i64>,
    end: Option<i64>,
) -> (usize, usize) {
    let clamp = |v: i64| -> usize {
        let v = if v < 0 { (v + len as i64).max(0) } else { v };
        v.min(len as i64) as usize
    };
    let s = start.map(clamp).unwrap_or(0);
    let e = end.map(clamp).unwrap_or(len);
    (s, e.max(s))
}

/// Extract an optional integer argument, treating a missing arg OR an
/// explicit `None` the same way (both mean "not given" — real CPython's
/// `str.find(sub, start=None)` etc. accept `None` as a valid stand-in for
/// "no bound").
pub(crate) fn opt_i64_arg(a: Option<&PyObjectRef>) -> Option<i64> {
    a.and_then(|v| {
        if matches!(&*v.borrow(), PyObject::None) {
            None
        } else {
            v.as_i64()
        }
    })
}

/// Shared core of `str.find`/`rfind`/`index`/`rindex` — operates on
/// CHARACTER (not byte) indices throughout, unlike a bare `str::find`/
/// `str::rfind` call (which return byte offsets — silently wrong as a
/// Python character index for any non-ASCII haystack), and properly
/// honors `start`/`end` (previously ignored entirely: `s.find(x, 5)`
/// searched from position 0 regardless of the `5`).
/// Extracts the `[char_start, char_end)` substring of `s`, returning the
/// resolved char-index `start` alongside it. Avoids materializing the
/// WHOLE string as `Vec<char>` (an O(n) allocation PLUS a per-char Unicode
/// decode) when `s` is pure ASCII, where byte index == char index, so a
/// plain byte slice is both correct and effectively free — falls back to
/// the `Vec<char>` approach only for genuinely non-ASCII strings, where
/// character and byte offsets can differ.
///
/// This isn't a micro-optimization: `startswith`/`endswith`/`find` called
/// with an explicit start index, IN A LOOP (a common manual-substring-scan
/// idiom), previously turned what should be a cheap O(1)-ish operation
/// into an O(n) allocation on EVERY call — confirmed via a direct repro
/// (200,000 `"a"*4000 .startswith(...)` calls: ~26 seconds before this
/// fix) that this made CPython's own `test_str.py`'s
/// `test_find_periodic_pattern` (which does exactly this ~4000 times per
/// check, 1000 checks) time out outright, once `random.choices` (a
/// separate, previously-missing function that test also depends on) was
/// added and let the test's real body run for the first time.
pub(crate) fn char_slice_with_start(
    s: &str,
    start: Option<i64>,
    end: Option<i64>,
) -> (usize, String) {
    if s.is_ascii() {
        let (st, en) = resolve_str_slice_bounds(s.len(), start, end);
        (st, s[st..en].to_string())
    } else {
        let chars: Vec<char> = s.chars().collect();
        let (st, en) = resolve_str_slice_bounds(chars.len(), start, end);
        (st, chars[st..en].iter().collect())
    }
}

pub(crate) fn str_find_impl(
    haystack: &str,
    needle: &str,
    start: Option<i64>,
    end: Option<i64>,
    reverse: bool,
) -> Option<usize> {
    let (s, sub) = char_slice_with_start(haystack, start, end);
    let found = if reverse {
        sub.rfind(needle)
    } else {
        sub.find(needle)
    };
    found.map(|byte_idx| s + sub[..byte_idx].chars().count())
}

/// Extract a raw byte slice out of a bytes-like `PyObjectRef` (`bytes`,
/// `bytearray`, or a `'B'`-typecode `array.array` — all three implement
/// real Python's buffer protocol as a flat byte sequence, matching e.g.
/// `b"x".startswith(bytearray(b"x"))` and `base64.b64encode(array.array
/// ('B', b"x"))`, both real, non-hypothetical idioms CPython's own test
/// suite exercises). Returns `None` for anything else (str, int, a
/// non-byte-typecode array, ...), which callers turn into a TypeError.
pub(crate) fn arg_bytes(v: &PyObjectRef) -> Option<Vec<u8>> {
    match &*v.borrow() {
        PyObject::Bytes(b) => Some(b.clone()),
        PyObject::ByteArray(b) => Some(b.clone()),
        PyObject::Array(arr)
            if arr.typecode == 'B' || arr.typecode == 'b' || arr.typecode == 'c' =>
        {
            Some(arr.data.iter().map(|&f| f as u8).collect())
        }
        PyObject::MemoryView { .. } => mv_tobytes(v).ok(),
        _ => None,
    }
}

/// `startswith`/`endswith` accept either a single bytes-like object or a
/// tuple of them — extracted here once rather than duplicated in both
/// methods. Non-bytes-like tuple members are silently dropped (matching
/// `filter_map`'s effect elsewhere in this file for the analogous `str`
/// case), not raised as a TypeError — this mirrors how the existing `str`
/// implementation already behaves via `.map(|x| x.str())` for its own
/// prefix/suffix tuple, i.e. permissive rather than strict.
pub(crate) fn extract_bytes_or_tuple(v: &PyObjectRef) -> Vec<Vec<u8>> {
    let items: Vec<PyObjectRef> = {
        let b = v.borrow();
        if let PyObject::Tuple(items) = &*b {
            items.clone()
        } else {
            drop(b);
            vec![v.clone()]
        }
    };
    items.iter().filter_map(arg_bytes).collect()
}

/// `bytearray`'s own string-ish methods (upper/split/strip/etc.) delegate
/// to the `bytes` method table above via `bytearray_delegate` (build a
/// temporary `bytes` from the bytearray's current contents, call the same
/// named method on it, done) rather than duplicating ~30 methods' worth of
/// byte-manipulation logic a second time. Real CPython's `bytearray`
/// methods return `bytearray` (not `bytes`) for the transformed result,
/// and e.g. `bytearray.split()` returns a list of `bytearray` — this
/// walks the `bytes`-method's result and converts any `Bytes` found
/// (directly, or nested inside a `List`/`Tuple` — covering `split`'s list
/// and `partition`'s tuple) back into `ByteArray`. Bools/ints (`count`,
/// `isalpha`, ...) pass through unchanged.
fn bytes_result_to_bytearray(v: PyObjectRef) -> PyObjectRef {
    let converted = {
        let b = v.borrow();
        match &*b {
            PyObject::Bytes(bytes) => Some(PyObjectRef::new(PyObject::ByteArray(bytes.clone()))),
            PyObject::List(items) => Some(py_list(
                items
                    .iter()
                    .map(|i| bytes_result_to_bytearray(i.clone()))
                    .collect(),
            )),
            PyObject::Tuple(items) => Some(PyObjectRef::imm(PyObject::Tuple(
                items
                    .iter()
                    .map(|i| bytes_result_to_bytearray(i.clone()))
                    .collect(),
            ))),
            _ => None,
        }
    };
    converted.unwrap_or(v)
}

/// Calls `bytes`'s implementation of `method_name` against a temporary
/// `bytes` snapshot of a `bytearray`'s current contents, forwarding the
/// rest of `args` unchanged, then converts the result back per
/// `bytes_result_to_bytearray`'s doc comment. See `bytearray`'s dispatch
/// arms below — one line each, all funneling through this.
pub(crate) fn bytearray_delegate(method_name: &str, args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if let PyObject::ByteArray(b) = &*args[0].borrow() {
        let temp = PyObjectRef::imm(PyObject::Bytes(b.clone()));
        let method = temp.borrow().get_attribute(method_name)?;
        let result = if let PyObject::BuiltinMethod { func, .. } = &*method.borrow() {
            let mut new_args = vec![temp.clone()];
            new_args.extend_from_slice(&args[1..]);
            func(&new_args)?
        } else {
            return Err(PyError::runtime_error(format!(
                "{}: bad method",
                method_name
            )));
        };
        Ok(bytes_result_to_bytearray(result))
    } else {
        Err(PyError::runtime_error(format!(
            "{} on non-bytearray",
            method_name
        )))
    }
}

/// Byte-slice analogue of `str_find_impl` — no unicode decoding needed
/// since `bytes` operates byte-for-byte.
pub(crate) fn bytes_find_impl(
    haystack: &[u8],
    needle: &[u8],
    start: Option<i64>,
    end: Option<i64>,
    reverse: bool,
) -> Option<usize> {
    let (s, e) = resolve_str_slice_bounds(haystack.len(), start, end);
    if s > e {
        return None;
    }
    let sub = &haystack[s..e];
    if needle.is_empty() {
        return Some(if reverse { e } else { s });
    }
    if needle.len() > sub.len() {
        return None;
    }
    if reverse {
        sub.windows(needle.len())
            .rposition(|w| w == needle)
            .map(|i| s + i)
    } else {
        sub.windows(needle.len())
            .position(|w| w == needle)
            .map(|i| s + i)
    }
}

/// Get the effective __slots__ for a type, checking the entire MRO.
/// Returns None if no __slots__ is defined anywhere in the hierarchy.
/// Look up a dunder method (e.g. `__len__`) on a type, walking its MRO —
/// unlike a direct `type_dict.get_str(name)` poke, this finds methods
/// defined on a base class, not just the instance's own leaf type.
pub(crate) fn lookup_dunder_via_mro(typ: &PyObjectRef, name: &str) -> Option<PyObjectRef> {
    let typ_ref = typ.borrow();
    if let PyObject::Type {
        dict: type_dict,
        mro,
        ..
    } = &*typ_ref
    {
        // Every class implicitly inherits `object`'s generic
        // __repr__/__eq__/__hash__/etc. Those must not preempt a class
        // with a native base (`class Foo(str): ...`) from getting the real
        // str/list/dict behavior for these — in CPython that behavior
        // comes from str/list/dict's own dunders sitting ahead of object
        // in the real mro; here, since the native base isn't literally a
        // PyObject::Type in mro, skip object's default instead, so the
        // native-backing fallback each of these call sites adds after a
        // None result gets a chance to run.
        let native_marker = type_dict.contains_key_str(NATIVE_BASE_MARKER);
        let skip_object_default = native_marker
            && matches!(
                name,
                "__repr__" | "__str__" | "__eq__" | "__ne__" | "__hash__"
            );
        // A migrated native type's OWN `__getitem__`/`__setitem__`/
        // `__delitem__` entries (e.g. `dict.__setitem__`, see
        // `NATIVE_VALUE_CTOR_KEY`'s doc comment) exist as an "escape hatch"
        // for EXPLICIT unbound-style access (`dict.__setitem__(x, k, v)`,
        // `super().__setitem__(k, v)` inside a subclass's own override) —
        // they must NOT preempt a native-base subclass's ordinary,
        // instance-level subscript dispatch, which already correctly
        // delegates to the native backing (and, for dict specifically,
        // consults `__missing__`) via each call site's own post-`None`
        // fallback. Skipping them here for ancestor-scan purposes (the
        // subclass's OWN override, if any, is still found by the
        // `type_dict.get_str(name)` check above, unaffected) restores that
        // fallback path for a subclass like `collections.Counter(dict)`
        // that overrides `__missing__` but not `__getitem__` itself —
        // confirmed via `Counter()["missing_key"]` regressing to a raw
        // `KeyError` (bypassing `__missing__` entirely) the instant `dict`
        // became a real `Type` with a real `__getitem__` newly sitting in
        // every dict-subclass's mro.
        let skip_native_dunder_hatch =
            native_marker && matches!(name, "__getitem__" | "__setitem__" | "__delitem__");
        // `object`'s own `__setattr__`/`__delattr__` (real, present in
        // `object.__dict__`) must NEVER preempt this ancestor walk finding a
        // GENUINE override further down — but there's no genuine override
        // to find below `object` itself, and `object`'s own native
        // implementation is a raw instance-dict poke with no descriptor
        // awareness, unlike STORE_ATTR/DELETE_ATTR's own separate
        // "check for a __set__/__delete__ descriptor" fallback (which runs
        // only when this function returns None). Finding `object`'s default
        // here instead of falling through to that fallback broke any
        // `property`-based read-only attribute on a class with no
        // `__setattr__` of its own (`xml.dom.minicompat.NodeList.length`'s
        // setter raising `NoModificationAllowedErr`, silently replaced by a
        // plain successful instance-dict write).
        let skip_object_setdelattr = matches!(name, "__setattr__" | "__delattr__");
        // Always check the type's OWN dict first, regardless of whether
        // `mro` is empty. For an ordinary user-defined class this is a
        // no-op (real mro-building always puts the class itself at
        // `mro[0]`, so this duplicates that first check harmlessly) — but
        // some native, hand-constructed `PyObject::Type`s (e.g. `dev.rs`'s
        // closure-built `StringIO`) set `mro` to just their BASE classes,
        // omitting themselves, since they're built ad hoc rather than via
        // the real class-creation/mro-linearization machinery. Without
        // this, such a type's OWN methods (its whole reason for existing)
        // were invisible to any dunder lookup that goes through this
        // function specifically (`__next__`/`__iter__`/etc. — plain
        // attribute access via `get_attribute_impl` checks the type's own
        // dict directly and was unaffected) — confirmed via `for line in
        // io.StringIO(...):`, `TypeError: 'instance' is not an iterator`.
        if let Some(v) = type_dict.get_str(name) {
            return Some(v.clone());
        }
        if mro.is_empty() {
            return None;
        }
        for base in mro.iter() {
            if let PyObject::Type {
                name: base_name,
                dict: base_dict,
                ..
            } = &*base.borrow()
            {
                if (skip_object_default || skip_object_setdelattr) && base_name == "object" {
                    continue;
                }
                if skip_native_dunder_hatch && base_dict.contains_key_str(NATIVE_VALUE_CTOR_KEY) {
                    continue;
                }
                if let Some(v) = base_dict.get_str(name) {
                    return Some(v.clone());
                }
            }
        }
        None
    } else {
        None
    }
}
