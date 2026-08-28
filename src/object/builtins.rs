// Split out of the former monolithic object.rs (see the file-splitting
// refactor's memory entry for context) — this file holds the ~79 standalone
// `pub fn builtin_*` free functions (the builtins module's actual
// implementations: print, len, isinstance, issubclass, format, iter/next,
// eval/exec, and so on).
use super::*;


mod print;
pub use print::builtin_print;
pub(crate) use print::{call_method_rebound, print_with_vm};

mod builtin_type;
pub use builtin_type::{builtin_type_of, type_new_builtin};
pub(crate) use builtin_type::{dict_arg_to_hashmap, get_primitive_type, seed_primitive_type_cache};

mod attribute;
pub use attribute::{builtin_ascii, builtin_bin, builtin_chr, builtin_delattr, builtin_exit, builtin_getattr, builtin_hasattr, builtin_hex, builtin_input, builtin_oct, builtin_ord, builtin_setattr};

mod int;
pub use int::{builtin_int, builtin_int_from_bytes};
pub(crate) use int::check_int_str_digit_limit;

mod numeric;
pub use numeric::{builtin_abs, builtin_divmod, builtin_pow, builtin_round};
pub(crate) use numeric::{bigint_gcd, bigint_mod_inverse, round_half_even_rat};

mod functional;
pub use functional::{builtin_all, builtin_any, builtin_breakpoint, builtin_callable, builtin_enumerate, builtin_max, builtin_min, builtin_sorted, builtin_sum};

mod iter;
pub use iter::{builtin_iter, builtin_len, builtin_next, builtin_range, builtin_reversed, list_iter_setstate, range_iter_setstate};
pub(crate) use iter::{iterable_length_hint, range_index_arg};

mod string;
pub use string::{builtin_bool, builtin_complex, builtin_float, builtin_format, builtin_repr, builtin_str, bytes_maketrans_builtin, str_maketrans_builtin};
pub(crate) use string::{bigint_to_float, check_int_to_str_limit, float_class_hex, float_fromhex, float_subclass_result, int_value_or_backing, ldexp_f64, python_bytes_repr, validate_underscores};

mod introspection;
pub use introspection::{builtin_dir, builtin_globals, builtin_hash, builtin_help, builtin_id, builtin_isinstance, builtin_issubclass, builtin_locals, builtin_object, builtin_open, builtin_slice, builtin_vars, call_bound_method, call_function_disposable};
pub(crate) use introspection::{exception_instance_repr, exception_instance_str, is_exception_type, path_arg_to_string};


pub fn builtin_list(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    // `list(iterable, extra)` is always wrong; keyword rejection happens in
    // call_function (where the real keyword list is visible), so a POSITIONAL
    // dict stays valid (`list({'a': 1})` -> `['a']`).
    if args.len() > 1 {
        return Err(PyError::type_error(format!(
            "list() takes at most 1 argument ({} given)",
            args.len()
        )));
    }
    if args.is_empty() {
        Ok(py_list(Vec::new()))
    } else {
        // Convert iterable to list
        let obj = args[0].borrow();
        match &*obj {
            PyObject::List(v) => Ok(py_list(v.clone())),
            PyObject::Tuple(v) => Ok(py_list(v.clone())),
            PyObject::Str(s) => {
                let items: Vec<PyObjectRef> = s.chars().map(|c| py_str(&c.to_string())).collect();
                Ok(py_list(items))
            }
            PyObject::Set(s) => Ok(py_list(s.to_vec())),
            _ => {
                drop(obj);
                // Try general iteration protocol via iter() + next() — any
                // error from `builtin_iter` must propagate AS-IS, not get
                // replaced with a generic "cannot convert" TypeError: an
                // object WITH `__iter__` whose call raises some OTHER
                // exception (real trigger: CPython's own `list_tests.py`'s
                // `test_constructor_exception_handling`, `class F: def
                // __iter__(self): raise KeyboardInterrupt`) had that
                // exception silently swallowed and replaced. `builtin_iter`
                // already produces the correct, more accurate message for
                // the genuinely-not-iterable case too (`'int' object is not
                // iterable`, matching real CPython exactly) — the previous
                // generic message here was actually a REGRESSION from that.
                let it = builtin_iter(&[args[0].clone()])?;
                let mut collected = Vec::new();
                if let Some(hint) = iterable_length_hint(&args[0]) {
                    if collected.try_reserve_exact(hint).is_err() {
                        return Err(PyError::memory_error("could not allocate list"));
                    }
                }
                loop {
                    match builtin_next(&[it.clone()]) {
                        Ok(val) => collected.push(val),
                        Err(PyError::StopIteration) => break,
                        Err(e) => return Err(e),
                    }
                }
                Ok(py_list(collected))
            }
        }
    }
}


pub fn builtin_deque(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    // `deque(iterable=None, maxlen=None)` — `maxlen` may come positionally
    // (`deque('abc', 3)`) or as a keyword (`deque('abc', maxlen=3)`); the
    // VM packs keywords into a trailing `PyObject::Dict`.
    let mut positional_maxlen: Option<PyObjectRef> = None;
    let mut iterable: Option<PyObjectRef> = None;
    let mut kw_maxlen: Option<PyObjectRef> = None;
    let mut iterable_seen = false;
    for arg in args.iter() {
        if let PyObject::Dict(d) = &*arg.borrow() {
            for (k, v) in d.items() {
                if k.str() == "maxlen" {
                    if kw_maxlen.is_some() || positional_maxlen.is_some() {
                        return Err(PyError::type_error(
                            "deque() got multiple values for argument 'maxlen'",
                        ));
                    }
                    kw_maxlen = Some(v);
                } else {
                    return Err(PyError::type_error(format!(
                        "deque() got an unexpected keyword argument '{}'",
                        k.str()
                    )));
                }
            }
        } else if !iterable_seen {
            iterable = Some(arg.clone());
            iterable_seen = true;
        } else if positional_maxlen.is_none() {
            positional_maxlen = Some(arg.clone());
        } else {
            return Err(PyError::type_error("deque() takes at most 2 arguments"));
        }
    }
    if positional_maxlen.is_some() && kw_maxlen.is_some() {
        return Err(PyError::type_error(
            "deque() got multiple values for argument 'maxlen'",
        ));
    }
    let maxlen_ref = positional_maxlen.or(kw_maxlen);
    let maxlen = if let Some(m) = maxlen_ref {
        // `maxlen=None` (positional or keyword) means UNBOUNDED.
        if matches!(&*m.borrow(), PyObject::None) {
            None
        } else {
            let n = m
                .as_i64()
                .ok_or_else(|| PyError::type_error("an integer is required"))?;
            if n < 0 {
                return Err(PyError::value_error("maxlen must be non-negative"));
            }
            Some(n as usize)
        }
    } else {
        None
    };
    let mut data: VecDeque<PyObjectRef> = VecDeque::new();
    if let Some(iter) = iterable {
        // Iterate through the real iterator protocol (NOT `__len__` +
        // `__getitem__`) — a lying source (`seq_tests.LyingTuple`) reports a
        // wrong `__len__` but a real iterator yields the true contents.
        let it = builtin_iter(&[iter])?;
        loop {
            match builtin_next(&[it.clone()]) {
                Ok(v) => {
                    data.push_back(v);
                    if let Some(maxlen) = maxlen {
                        if data.len() > maxlen {
                            data.pop_front();
                        }
                    }
                }
                Err(PyError::StopIteration) => break,
                Err(e) => return Err(e),
            }
        }
    }
    Ok(py_deque(data, maxlen))
}


pub fn builtin_tuple(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Ok(py_tuple(Vec::new()));
    }
    if args.len() > 1 {
        return Err(PyError::type_error(format!(
            "tuple expected at most 1 argument, got {}",
            args.len()
        )));
    }
    // `tuple(t)` returns the SAME tuple (identity optimization,
    // test_tuple::test_constructors: `t0_3 is tuple(t0_3)`).
    if let PyObject::Tuple(_) = &*args[0].borrow() {
        return Ok(args[0].clone());
    }
    // `tuple(x)` accepts ANY iterable in real Python, not just the handful
    // of native container shapes this used to special-case — e.g.
    // `tuple(map(...))`, generators, custom `__iter__` objects all raised
    // "cannot convert '...' to tuple" instead of actually iterating. Same
    // general fix already applied to `set()`/`str.join`: materialize
    // through the real iterator protocol. Keep the List/Tuple fast paths
    // (avoid a full iterator round-trip for the overwhelmingly common
    // cases) and the Str fast path (per-character, not per-codepoint-int).
    {
        let obj = args[0].borrow();
        match &*obj {
            PyObject::List(v) => return Ok(py_tuple(v.clone())),
            PyObject::Tuple(v) => return Ok(py_tuple(v.clone())),
            PyObject::Str(s) => {
                let items: Vec<PyObjectRef> = s.chars().map(|c| py_str(&c.to_string())).collect();
                return Ok(py_tuple(items));
            }
            _ => {}
        }
    }
    let iterator = builtin_iter(&[args[0].clone()])?;
    let mut items: Vec<PyObjectRef> = Vec::new();
    if let Some(hint) = iterable_length_hint(&args[0]) {
        if items.try_reserve_exact(hint).is_err() {
            return Err(PyError::memory_error("could not allocate tuple"));
        }
    }
    loop {
        match builtin_next(&[iterator.clone()]) {
            Ok(v) => items.push(v),
            Err(PyError::StopIteration) => break,
            Err(e) => return Err(e),
        }
    }
    Ok(py_tuple(items))
}


/// `dict.__repr__` — a per-type repr so `type(x).__repr__` differs across
/// native container types (CPython's pprint dispatches on
/// `type(object).__repr__`; a shared identity repr made dict and deque
/// collide and route dicts through pprint's deque formatter).
pub fn builtin_dict_repr(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error("__repr__ requires 1 argument"));
    }
    Ok(py_str(&args[0].repr()))
}


/// `deque.__repr__` — see `builtin_dict_repr`.
pub fn builtin_deque_repr(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error("__repr__ requires 1 argument"));
    }
    Ok(py_str(&args[0].repr()))
}


// Per-type `__repr__` functions. Each native container/type must have its
// OWN `__repr__` function object: CPython's pprint dispatches on
// `type(object).__repr__` (and other code compares `x.__repr__ is
// y.__repr__`), so sharing one identity repr across all types made them
// indistinguishable (dict routed through pprint's deque formatter, etc.).
macro_rules! native_repr_fn {
    ($name:ident) => {

        pub fn $name(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
            if args.is_empty() {
                return Err(PyError::type_error("__repr__ requires 1 argument"));
            }
            Ok(py_str(&args[0].repr()))
        }
    };
}
native_repr_fn!(builtin_list_repr);
native_repr_fn!(builtin_tuple_repr);
native_repr_fn!(builtin_str_repr);
native_repr_fn!(builtin_bytes_repr);
native_repr_fn!(builtin_bytearray_repr);
native_repr_fn!(builtin_int_repr);
native_repr_fn!(builtin_float_repr);
native_repr_fn!(builtin_complex_repr);
native_repr_fn!(builtin_bool_repr);
native_repr_fn!(builtin_set_repr);
native_repr_fn!(builtin_frozenset_repr);
native_repr_fn!(builtin_slice_repr);


pub fn builtin_dict(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Ok(py_dict());
    }
    let mut d = PyDict::new();
    let arg = args[0].borrow();
    match &*arg {
        PyObject::Dict(other) => {
            for (k, v) in other.items() {
                d.set(k, v)?;
            }
        }
        _ => {
            drop(arg);
            // Mapping-like objects (anything with a `.keys()` method —
            // notably dict *subclass* instances like Counter/defaultdict/
            // ChainMap, which are never literally PyObject::Dict) are
            // copied key-by-key, matching real `dict(mapping)` semantics.
            // Only objects without `.keys()` fall back to being treated
            // as an iterable of (key, value) pairs.
            let type_name = args[0].borrow().type_name();
            let is_view = matches!(type_name.as_str(), "dict_items" | "dict_keys" | "dict_values" | "KeysView" | "ItemsView" | "ValuesView" | "MappingView");
            let keys_method = if is_view { None } else { args[0].borrow().get_attribute("keys").ok() };
            if let Some(keys_raw) = keys_method {
                let keys_iterable = call_bound_method(keys_raw, args[0].clone(), vec![])?;
                let it = builtin_iter(&[keys_iterable])?;
                loop {
                    match builtin_next(&[it.clone()]) {
                        Ok(key) => {
                            let value = py_getitem(&args[0], &key)?;
                            d.set(key, value)?;
                        }
                        Err(PyError::StopIteration) => break,
                        Err(e) => return Err(e),
                    }
                }
            } else {
                // An iterable of (key, value) pairs
                let it = builtin_iter(&[args[0].clone()])?;
                loop {
                    match builtin_next(&[it.clone()]) {
                        Ok(pair) => {
                            let pair_b = pair.borrow();
                            let items: Vec<PyObjectRef> = match &*pair_b {
                                PyObject::Tuple(v) | PyObject::List(v) => v.clone(),
                                _ => return Err(PyError::type_error("cannot convert dictionary update sequence element to a sequence")),
                            };
                            if items.len() != 2 {
                                return Err(PyError::value_error(format!(
                                    "dictionary update sequence element has length {}; 2 is required", items.len()
                                )));
                            }
                            drop(pair_b);
                            d.set(items[0].clone(), items[1].clone())?;
                        }
                        Err(PyError::StopIteration) => break,
                        Err(e) => return Err(e),
                    }
                }
            }
        }
    }
    // Real `dict()` only ever takes ONE positional argument
    // (`dict(mapping_or_iterable=(), /, **kwargs)`) — this project's own
    // calling convention packs a call's keyword arguments into ONE extra
    // trailing dict appended to `args` (see `call_function`'s
    // `PyObject::BuiltinFunction` handling), so any `args[1..]` seen here is
    // never a second genuine positional, always that packed kwargs dict.
    // Merge it on top (kwargs win on key collisions, matching real
    // `dict(d, **kwargs)` semantics) — needed for real stdlib code like
    // `argparse.py`'s `dict(kwargs, dest=dest, option_strings=[])`, which
    // previously silently dropped `dest`/`option_strings` entirely.
    for extra in &args[1..] {
        if let PyObject::Dict(other) = &*extra.borrow() {
            for (k, v) in other.items() {
                d.set(k, v)?;
            }
        }
    }
    Ok(PyObjectRef::new(PyObject::Dict(Box::new(d))))
}


pub fn builtin_set(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Ok(py_set());
    }
    // `set(x)` accepts any iterable in real Python — dicts (yielding keys),
    // generators, custom `__iter__` objects, etc., not just the handful of
    // native container shapes this used to special-case (which also had
    // its own bug: `set("abc")` produced a set of *codepoint ints* instead
    // of single-character strings, since the Str arm bypassed the normal
    // per-character string wrapping every other string-iteration path
    // uses). Materializing through the real iterator protocol fixes both
    // at once and matches the same general fix already applied to
    // `str.join`.
    let iterator = crate::object::builtin_iter(&[args[0].clone()])?;
    let mut elts: Vec<PyObjectRef> = Vec::new();
    loop {
        match crate::object::builtin_next(&[iterator.clone()]) {
            Ok(v) => elts.push(v),
            Err(PyError::StopIteration) => break,
            Err(e) => return Err(e),
        }
    }
    Ok(PyObjectRef::new(PyObject::Set(PySet::from_vec(elts)?)))
}


pub fn builtin_bytes(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        Ok(PyObjectRef::imm(PyObject::Bytes(Vec::new())))
    } else {
        // Buffer protocol: try memoryview first (e.g. bytes(MyBuffer()) where MyBuffer defines __buffer__)
        {
            let obj = args[0].clone();
            if let Ok(mv) = crate::object::builtin_memoryview(&[obj.clone()]) {
                if let Ok(bytes) = crate::object::mv_tobytes(&mv) {
                    return Ok(PyObjectRef::imm(PyObject::Bytes(bytes)));
                }
            }
        }
        // PickleBuffer and memoryview are bytes-like via buffer protocol
        {
            let b = args[0].borrow();
            if let PyObject::Instance { typ, dict } = &*b {
                let is_pb = if let PyObject::Type { name, .. } = &*typ.borrow() {
                    name == "PickleBuffer"
                } else {
                    false
                };
                if is_pb {
                    let released = dict
                        .get("_released")
                        .map(|v| v.truthy())
                        .unwrap_or(false);
                    if released {
                        return Err(PyError::value_error(
                            "operation forbidden on released PickleBuffer object",
                        ));
                    }
                    let underlying = dict.get("_obj").cloned().unwrap_or_else(py_none);
                    drop(b);
                    return builtin_bytes(&[underlying]);
                }
            }
            if let PyObject::MemoryView { released, .. } = &*b {
                if *released {
                    return Err(PyError::value_error(
                        "operation forbidden on released memoryview object",
                    ));
                }
                // fall through to dedicated memoryview handling below after drop
            }
        }
        // memoryview -> bytes is a direct tobytes copy
        if matches!(&*args[0].borrow(), PyObject::MemoryView { .. }) {
            let bytes = crate::object::mv_tobytes(&args[0])?;
            return Ok(PyObjectRef::imm(PyObject::Bytes(bytes)));
        }
        let obj = args[0].borrow();
        match &*obj {
            // Same fix as `bytearray(n)` above: `bytes(n)` zero-fills a
            // buffer of length `n`, it doesn't wrap `n` as a single byte
            // value.
            PyObject::Int(i) => {
                let n = i
                    .to_i64()
                    .ok_or_else(|| PyError::value_error("bytes() argument must be non-negative"))?;
                if n < 0 {
                    return Err(PyError::value_error(
                        "bytes() argument must be non-negative",
                    ));
                }
                Ok(PyObjectRef::imm(PyObject::Bytes(vec![0u8; n as usize])))
            }
            PyObject::Bytes(b) => Ok(PyObjectRef::imm(PyObject::Bytes(b.clone()))),
            PyObject::ByteArray(b) => Ok(PyObjectRef::imm(PyObject::Bytes(b.clone()))),
            PyObject::Str(s) => Ok(PyObjectRef::imm(PyObject::Bytes(s.as_bytes().to_vec()))),
            PyObject::List(v) => {
                let mut result = Vec::new();
                for item in v {
                    let item = item.borrow();
                    if let PyObject::Int(i) = &*item {
                        let n = i.to_i64().ok_or_else(|| {
                            PyError::value_error("bytes() requires int in range 0-255")
                        })?;
                        if n < 0 || n > 255 {
                            return Err(PyError::value_error(
                                "bytes() requires int in range 0-255",
                            ));
                        }
                        result.push(n as u8);
                    } else {
                        return Err(PyError::type_error(
                            "bytes() argument must be an integer or iterable",
                        ));
                    }
                }
                Ok(PyObjectRef::imm(PyObject::Bytes(result)))
            }
            PyObject::Tuple(v) => {
                let mut result = Vec::new();
                for item in v {
                    let item = item.borrow();
                    if let PyObject::Int(i) = &*item {
                        let n = i.to_i64().ok_or_else(|| {
                            PyError::value_error("bytes() requires int in range 0-255")
                        })?;
                        if n < 0 || n > 255 {
                            return Err(PyError::value_error(
                                "bytes() requires int in range 0-255",
                            ));
                        }
                        result.push(n as u8);
                    } else {
                        return Err(PyError::type_error(
                            "bytes() argument must be an integer or iterable",
                        ));
                    }
                }
                Ok(PyObjectRef::imm(PyObject::Bytes(result)))
            }
            PyObject::Set(items) | PyObject::FrozenSet(items) => {
                let mut result = Vec::new();
                for item in items.to_vec() {
                    let item_b = item.borrow();
                    if let PyObject::Int(i) = &*item_b {
                        let n = i.to_i64().ok_or_else(|| {
                            PyError::value_error("bytes() requires int in range 0-255")
                        })?;
                        if n < 0 || n > 255 {
                            return Err(PyError::value_error(
                                "bytes() requires int in range 0-255",
                            ));
                        }
                        result.push(n as u8);
                    } else {
                        return Err(PyError::type_error(
                            "bytes() argument must be integer or iterable",
                        ));
                    }
                }
                Ok(PyObjectRef::imm(PyObject::Bytes(result)))
            }
            _ => {
                drop(obj);
                // Same fix as `builtin_list`'s matching site: propagate
                // `builtin_iter`'s error as-is rather than replacing it with
                // a generic message (swallowing a real exception raised
                // from inside a custom `__iter__`).
                let it = builtin_iter(&[args[0].clone()])?;
                let mut result = Vec::new();
                loop {
                    let item = match builtin_next(&[it.clone()]) {
                        Ok(val) => val,
                        Err(PyError::StopIteration) => break,
                        Err(e) => return Err(e),
                    };
                    let item_b = item.borrow();
                    if let PyObject::Int(i) = &*item_b {
                        let n = i.to_i64().ok_or_else(|| {
                            PyError::value_error("bytes() requires int in range 0-255")
                        })?;
                        if n < 0 || n > 255 {
                            return Err(PyError::value_error(
                                "bytes() requires int in range 0-255",
                            ));
                        }
                        result.push(n as u8);
                    } else {
                        return Err(PyError::type_error(
                            "bytes() argument must be an integer or iterable",
                        ));
                    }
                }
                Ok(PyObjectRef::imm(PyObject::Bytes(result)))
            }
        }
    }
}


/// bytes.fromhex(string) -> bytes
///
/// Create a bytes object from a string of hexadecimal digits.
pub fn builtin_bytes_fromhex(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error(
            "bytes.fromhex() takes exactly 1 argument (0 given)",
        ));
    }
    let s = args[0].str();
    // Remove spaces (CPython allows spaces in the hex string)
    let s = s.replace(' ', "");
    if s.len() % 2 != 0 {
        return Err(PyError::value_error("hex string must be of even length"));
    }
    let mut result = Vec::with_capacity(s.len() / 2);
    let bytes = s.as_bytes();
    for chunk in bytes.chunks(2) {
        let hex_pair = std::str::from_utf8(chunk)
            .map_err(|_| PyError::value_error("non-hexadecimal number found"))?;
        let byte = u8::from_str_radix(hex_pair, 16).map_err(|_| {
            PyError::value_error(format!(
                "non-hexadecimal number found in fromhex() arg at position {}",
                s.find(hex_pair).unwrap_or(0)
            ))
        })?;
        result.push(byte);
    }
    Ok(PyObjectRef::imm(PyObject::Bytes(result)))
}


pub fn builtin_bytearray(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        Ok(PyObjectRef::new(PyObject::ByteArray(Vec::new())))
    } else {
        let obj = args[0].borrow();
        match &*obj {
            // Real `bytearray(n)` (a single non-negative int argument)
            // creates a zero-filled buffer of length `n` — NOT a
            // single-element buffer holding the byte value `n` (that's
            // `bytes([n])`, a completely different construction). This
            // matched the length-1 anti-pattern instead, silently breaking
            // the extremely common "pre-allocate an I/O buffer"
            // idiom — found via `struct.pack_into`'s own doctest-style
            // idiom `bytearray(10)`.
            PyObject::Int(i) => {
                let n = i.to_i64().ok_or_else(|| {
                    PyError::value_error("bytearray() argument must be non-negative")
                })?;
                if n < 0 {
                    return Err(PyError::value_error(
                        "bytearray() argument must be non-negative",
                    ));
                }
                Ok(PyObjectRef::new(PyObject::ByteArray(vec![0u8; n as usize])))
            }
            PyObject::Bytes(b) => Ok(PyObjectRef::new(PyObject::ByteArray(b.clone()))),
            PyObject::ByteArray(b) => Ok(PyObjectRef::new(PyObject::ByteArray(b.clone()))),
            PyObject::Str(s) => Ok(PyObjectRef::new(PyObject::ByteArray(s.as_bytes().to_vec()))),
            PyObject::List(v) => {
                let mut result = Vec::new();
                for item in v {
                    let item = item.borrow();
                    if let PyObject::Int(i) = &*item {
                        let n = i.to_i64().ok_or_else(|| {
                            PyError::value_error("bytearray() requires int in range 0-255")
                        })?;
                        if n < 0 || n > 255 {
                            return Err(PyError::value_error(
                                "bytearray() requires int in range 0-255",
                            ));
                        }
                        result.push(n as u8);
                    } else {
                        return Err(PyError::type_error(
                            "bytearray() argument must be an integer or iterable",
                        ));
                    }
                }
                Ok(PyObjectRef::new(PyObject::ByteArray(result)))
            }
            PyObject::Tuple(v) => {
                let mut result = Vec::new();
                for item in v {
                    let item = item.borrow();
                    if let PyObject::Int(i) = &*item {
                        let n = i.to_i64().ok_or_else(|| {
                            PyError::value_error("bytearray() requires int in range 0-255")
                        })?;
                        if n < 0 || n > 255 {
                            return Err(PyError::value_error(
                                "bytearray() requires int in range 0-255",
                            ));
                        }
                        result.push(n as u8);
                    } else {
                        return Err(PyError::type_error(
                            "bytearray() argument must be an integer or iterable",
                        ));
                    }
                }
                Ok(PyObjectRef::new(PyObject::ByteArray(result)))
            }
            PyObject::Range { .. } => {
                // Any iterable of ints (range, generators, custom __iter__)
                // is valid (test_pprint: bytearray(range(5))).
                drop(obj);
                let it = builtin_iter(&[args[0].clone()])?;
                let mut result = Vec::new();
                loop {
                    match builtin_next(&[it.clone()]) {
                        Ok(item) => {
                            let n = item.as_i64().ok_or_else(|| {
                                PyError::value_error("bytearray() requires int in range 0-255")
                            })?;
                            if n < 0 || n > 255 {
                                return Err(PyError::value_error(
                                    "bytearray() requires int in range 0-255",
                                ));
                            }
                            result.push(n as u8);
                        }
                        Err(PyError::StopIteration) => break,
                        Err(e) => return Err(e),
                    }
                }
                Ok(PyObjectRef::new(PyObject::ByteArray(result)))
            }
            _ => Err(PyError::type_error(format!(
                "cannot convert '{}' to bytearray",
                obj.type_name()
            ))),
        }
    }
}


pub fn builtin_frozenset(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        Ok(PyObjectRef::imm(PyObject::FrozenSet(PySet::new())))
    } else {
        let obj = args[0].borrow();
        match &*obj {
            PyObject::Set(s) => Ok(PyObjectRef::imm(PyObject::FrozenSet(s.clone()))),
            PyObject::FrozenSet(s) => Ok(PyObjectRef::imm(PyObject::FrozenSet(s.clone()))),
            PyObject::List(v) => {
                let mut set = PySet::new();
                for item in v {
                    set.add(item.clone())?;
                }
                Ok(PyObjectRef::imm(PyObject::FrozenSet(set)))
            }
            PyObject::Tuple(v) => {
                let mut set = PySet::new();
                for item in v {
                    set.add(item.clone())?;
                }
                Ok(PyObjectRef::imm(PyObject::FrozenSet(set)))
            }
            PyObject::Str(s) => {
                let mut set = PySet::new();
                for ch in s.chars() {
                    set.add(py_str(&ch.to_string()))?;
                }
                Ok(PyObjectRef::imm(PyObject::FrozenSet(set)))
            }
            PyObject::Bytes(b) => {
                let mut set = PySet::new();
                for &byte in b {
                    set.add(py_int(byte as i64))?;
                }
                Ok(PyObjectRef::imm(PyObject::FrozenSet(set)))
            }
            PyObject::Range { .. } => {
                drop(obj);
                let it = builtin_iter(&[args[0].clone()])?;
                let mut set = PySet::new();
                loop {
                    match builtin_next(&[it.clone()]) {
                        Ok(item) => set.add(item.clone())?,
                        Err(PyError::StopIteration) => break,
                        Err(e) => return Err(e),
                    }
                }
                Ok(PyObjectRef::imm(PyObject::FrozenSet(set)))
            }
            _ => Err(PyError::type_error(format!(
                "cannot convert '{}' to frozenset",
                obj.type_name()
            ))),
        }
    }
}
