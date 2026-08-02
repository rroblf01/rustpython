// Split out of the former monolithic object.rs (see the file-splitting
// refactor's memory entry for context) — this file holds the VM-pointer
// thread-locals, the class registry backing `type.__subclasses__()`, the
// `py_*` constructor helpers, and the `%`-operator string interpolation
// implementation.
use super::*;

thread_local! {
    pub static VM_PTR: std::cell::RefCell<Option<*mut crate::vm::VirtualMachine>> = std::cell::RefCell::new(None);
}

thread_local! {
    pub static INT_MAX_STR_DIGITS: std::cell::Cell<i64> = const { std::cell::Cell::new(4300) };
}

/// Safely access the current VM via VM_PTR.
///
/// Returns `Err(runtime_error)` if no VM is active.
/// The single `unsafe` dereference for VM_PTR access lives here;
/// all callers use this safe wrapper instead of inline `unsafe { &*ptr }`.
pub fn with_vm_mut<F, R>(f: F) -> PyResult<R>
where
    F: FnOnce(&mut crate::vm::VirtualMachine) -> R,
{
    VM_PTR.with(|p| {
        let opt = p.borrow();
        if let Some(ptr) = *opt {
            // SAFETY:
            // - VM_PTR is set in `VirtualMachine::execute()` before execution begins
            //   and remains valid for the duration of the call.
            // - It is only set on the current thread (thread_local!).
            // - The pointer is cleared after execution completes.
            // - Therefore, while we are inside a builtin function being called by the VM,
            //   the pointer is guaranteed to point to a live VirtualMachine.
            let vm = unsafe { &mut *ptr };
            Ok(f(vm))
        } else {
            Err(PyError::runtime_error("no active VM"))
        }
    })
}

thread_local! {
    // Every user-defined class ever built (registered from
    // `VirtualMachine::default_build_class`), kept alive for the process's
    // lifetime — backs `type.__subclasses__()`. Real CPython tracks this via
    // a weak-ref list on each class, updated as subclasses are created/GC'd;
    // we don't have weak refs on this object model and classes are rarely
    // freed in practice for typical scripts, so a plain append-only registry
    // is a reasonable trade-off (leaks class objects, matching the process
    // lifetimes this interpreter actually runs for).
    static CLASS_REGISTRY: std::cell::RefCell<Vec<PyObjectRef>> = std::cell::RefCell::new(Vec::new());
}

pub(crate) fn register_class(cls: &PyObjectRef) {
    CLASS_REGISTRY.with(|r| r.borrow_mut().push(cls.clone()));
}

/// Find a registered class by its `__module__`/`__name__` qualified name —
/// backs the native pickle's subclass reconstruction. Searches the global
/// class registry (NOT `sys.modules`, which is VM-relative and unreliable
/// when the active VM pointer is a transient disposable one) for a `Type`
/// whose own dict matches both fields.
pub(crate) fn find_class_by_qualified_name(module: &str, name: &str) -> Option<PyObjectRef> {
    CLASS_REGISTRY.with(|r| {
        for cls in r.borrow().iter() {
            if let PyObject::Type { name: tname, dict, .. } = &*cls.borrow() {
                if tname != name {
                    continue;
                }
                let module_name = dict.get_str("__module__").map(|m| m.str()).unwrap_or_default();
                if module_name == module {
                    return Some(cls.clone());
                }
            }
        }
        None
    })
}

/// Direct (non-transitive) subclasses of `cls`, in registration order —
/// backs `type.__subclasses__()`.
pub(crate) fn direct_subclasses_of(cls: &PyObjectRef) -> Vec<PyObjectRef> {
    if std::env::var("RPY_DEBUG_SUBCLASSES").is_ok() {
        CLASS_REGISTRY.with(|r| eprintln!("SUBCLASSES_CALL registry_len={}", r.borrow().len()));
    }
    CLASS_REGISTRY.with(|r| {
        r.borrow()
            .iter()
            .filter(|candidate| {
                if let PyObject::Type { bases, .. } = &*candidate.borrow() {
                    bases.iter().any(|b| b.is(cls))
                } else {
                    false
                }
            })
            .cloned()
            .collect()
    })
}

pub fn py_int(i: impl Into<BigInt>) -> PyObjectRef {
    let big = i.into();
    if let Some(n) = big.to_i64() {
        return PyObjectRef::SmallInt(n);
    }
    PyObjectRef::imm(PyObject::Int(big))
}

pub fn py_bool(b: bool) -> PyObjectRef {
    PyObjectRef::SmallBool(b)
}

pub fn py_none() -> PyObjectRef {
    PyObjectRef::None
}

thread_local! {
    // The canonical `NotImplemented` singleton — seeded once from
    // `create_builtins()` (the same object bound to the `NotImplemented`
    // global name), so callers elsewhere (e.g. `object.__eq__`/`__ne__`'s
    // own native implementations, which are bare `fn` pointers with no way
    // to capture it from the enclosing scope) can return the SAME object,
    // preserving `result is NotImplemented`-style identity checks in
    // Python code. Mirrors `PRIMITIVE_TYPE_CACHE`'s seed-once-use-everywhere
    // pattern.
    static NOT_IMPLEMENTED_SINGLETON: std::cell::RefCell<Option<PyObjectRef>> = std::cell::RefCell::new(None);
}

pub(crate) fn seed_not_implemented(v: PyObjectRef) {
    NOT_IMPLEMENTED_SINGLETON.with(|c| *c.borrow_mut() = Some(v));
}

/// The `NotImplemented` singleton — panics if called before
/// `create_builtins()` has seeded it (i.e. before any VM exists at all).
pub fn py_not_implemented() -> PyObjectRef {
    NOT_IMPLEMENTED_SINGLETON.with(|c| c.borrow().clone().expect("NotImplemented singleton not yet seeded"))
}

/// Convert a Python object to a PySet by checking common iterable types.
/// Used as a replacement for the non-existent `py_set_from_iter`.
pub fn convert_to_set(obj: &PyObjectRef) -> PyResult<PySet> {
    let borrowed = obj.borrow();
    match &*borrowed {
        PyObject::Set(s) => Ok(s.clone()),
        PyObject::FrozenSet(s) => Ok(s.clone()),
        PyObject::List(v) => Ok(PySet::from_vec(v.clone())?),
        PyObject::Tuple(items) => Ok(PySet::from_vec(items.clone())?),
        PyObject::Str(s) => {
            let chars: Vec<PyObjectRef> = s.chars().map(|c| py_str(&c.to_string())).collect();
            Ok(PySet::from_vec(chars)?)
        }
        _ => Err(PyError::type_error(format!(
            "cannot convert '{}' to set", borrowed.type_name()
        ))),
    }
}

pub fn py_float(f: f64) -> PyObjectRef {
    // Use inline SmallFloat to avoid Rc + heap alloc
    PyObjectRef::SmallFloat(f)
}

pub fn py_str(s: &str) -> PyObjectRef {
    // Use inline SmallStr for strings < 16 bytes to avoid Rc + heap alloc
    if let Some(small) = SmallStr::new(s) {
        return PyObjectRef::SmallStr(small);
    }
    PyObjectRef::imm(PyObject::Str(compact_str::CompactString::from(s)))
}

pub fn py_list(items: Vec<PyObjectRef>) -> PyObjectRef {
    PyObjectRef::new(PyObject::List(items))
}

pub fn py_deque(data: VecDeque<PyObjectRef>, maxlen: Option<usize>) -> PyObjectRef {
    PyObjectRef::new(PyObject::Deque { data, maxlen })
}

pub fn py_tuple(items: Vec<PyObjectRef>) -> PyObjectRef {
    PyObjectRef::imm(PyObject::Tuple(items))
}

pub fn py_dict() -> PyObjectRef {
    PyObjectRef::new(PyObject::Dict(Box::new(PyDict::new())))
}

pub fn py_set() -> PyObjectRef {
    PyObjectRef::new(PyObject::Set(PySet::new()))
}

/// printf-style string interpolation (% operator)
pub(crate) fn string_interpolate(fmt: &str, arg: &PyObjectRef) -> Result<String, String> {
    let mut result = String::new();
    let mut chars = fmt.chars();

    // Handle tuple arguments: consume one element per format spec. Real
    // CPython's `%` operator does this ONLY for a tuple RHS — a list (or
    // any other single non-mapping value) is always used as-is for a
    // single conversion, never unpacked positionally. This used to also
    // treat `PyObject::List` the same as a tuple, so `"%r" % some_list`
    // silently dropped every element past the first `%`-spec instead of
    // formatting the whole list — confirmed via a general, non-Django repro
    // (`"%r" % self._tests` inside a `__repr__`, `self._tests` a 3-element
    // list, printing only the first element's repr instead of the whole
    // list's) that also masked an unrelated real bug during Django/unittest
    // bisecting (looked like a lost/dropped list element, was actually this
    // formatting bug misrepresenting a correctly-3-element list as 1).
    let mut arg_iter: Option<Box<dyn Iterator<Item = PyObjectRef>>> = None;
    let arg0 = arg.clone();
    {
        let obj = arg0.borrow();
        match &*obj {
            PyObject::Tuple(items) => {
                let vec = items.clone();
                let iter = vec.into_iter();
                arg_iter = Some(Box::new(iter));
            }
            _ => {}
        }
    }
    // Helper: get next arg (consume from tuple iterator, or always use the single arg)
    let mut get_arg = || -> PyObjectRef {
        if let Some(ref mut it) = arg_iter {
            it.next().unwrap_or_else(|| py_str(""))
        } else {
            arg.clone()
        }
    };

    while let Some(ch) = chars.next() {
        if ch == '%' {
            // Parse an optional mapping key: %(name)s pulls "name" out of a
            // dict argument instead of consuming positionally.
            let mut mapping_key: Option<String> = None;
            if chars.clone().next() == Some('(') {
                chars.next(); // consume '('
                let mut key = String::new();
                loop {
                    match chars.next() {
                        Some(')') => break,
                        Some(c) => key.push(c),
                        None => return Err("incomplete format key".to_string()),
                    }
                }
                mapping_key = Some(key);
            }
            // Parse optional width specifier (e.g., %03o, %02d, %3s, %4d)
            let mut width: Option<usize> = None;
            // Check for flags (only '0' flag supported)
            let mut flags = String::new();
            let peek = chars.clone();
            if let Some(c) = peek.as_str().chars().next() {
                if c == '0' {
                    flags.push('0');
                    chars.next();
                }
            }
            // Parse width (digits)
            let mut width_str = String::new();
            loop {
                let mut peek2 = chars.clone();
                match peek2.next() {
                    Some(c) if c.is_ascii_digit() => { width_str.push(c); chars.next(); }
                    _ => break,
                }
            }
            if !width_str.is_empty() {
                let w = width_str.parse::<usize>().map_err(|_| "invalid width".to_string())?;
                // A width like `sys.maxsize + 1` parses into a valid `usize`
                // but then panics trying to actually pad a string/number out
                // to that length — real CPython raises `ValueError` instead
                // (`test_str.py::test_formatting_huge_width`: `"%{}f" %
                // (sys.maxsize + 1)`). Same cap as the `.precision` check
                // just below, for consistency.
                if w > 1000 { return Err("width too big".to_string()); }
                width = Some(w);
            }
            // Parse optional `.precision` (e.g. `%.3f`, `%6.2f`) — this was
            // entirely unparsed before, so any precision-qualified float
            // format (an extremely common idiom for printing durations/
            // percentages/rounded numbers — real trigger: CPython 3.14's
            // own `unittest/runner.py`, `"%.3fs" % elapsed`) hit the literal
            // `.` as if it were an (unsupported) conversion character.
            let mut precision: Option<usize> = None;
            if chars.clone().next() == Some('.') {
                chars.next();
                let mut prec_str = String::new();
                loop {
                    let mut peek3 = chars.clone();
                    match peek3.next() {
                        Some(c) if c.is_ascii_digit() => { prec_str.push(c); chars.next(); }
                        _ => break,
                    }
                }
                precision = Some(if prec_str.is_empty() { 0 } else {
                    let p = prec_str.parse::<usize>().map_err(|_| "invalid precision".to_string())?;
                    // See the matching `width` cap above — a precision this
                    // large parses fine but panics Rust's own `format!`
                    // machinery when actually used (`test_str.py::
                    // test_formatting_huge_precision`: `"%.{}f" %
                    // (sys.maxsize + 1)`).
                    if p > 1000 { return Err("precision too big".to_string()); }
                    p
                });
            }

            match chars.next() {
                None => return Err("incomplete format: trailing %".to_string()),
                Some('%') => result.push('%'),
                Some(conv @ 's') | Some(conv @ 'r') | Some(conv @ 'f') | Some(conv @ 'd') | Some(conv @ 'i')
                | Some(conv @ 'o') | Some(conv @ 'x') | Some(conv @ 'X') | Some(conv @ 'c') => {
                    let raw = if let Some(ref key) = mapping_key {
                        let obj = arg.borrow();
                        match &*obj {
                            PyObject::Dict(d) => d.get(&py_str(key)).ok().flatten()
                                .ok_or_else(|| format!("'{}'", key))?,
                            _ => return Err("format requires a mapping".to_string()),
                        }
                    } else {
                        get_arg()
                    };

                    let formatted = match conv {
                        's' => raw.str(),
                        'r' => raw.repr(),
                        'f' => {
                            let f = raw.as_f64().unwrap_or(0.0);
                            format!("{:.prec$}", f, prec = precision.unwrap_or(6))
                        }
                        'd' | 'i' => {
                            if let Some(i) = raw.as_i64() {
                                i.to_string()
                            } else {
                                "0".to_string()
                            }
                        }
                        'o' => {
                            if let Some(i) = raw.as_i64() {
                                format!("{:o}", i)
                            } else {
                                "0".to_string()
                            }
                        }
                        'x' => {
                            if let Some(i) = raw.as_i64() {
                                format!("{:x}", i)
                            } else {
                                "0".to_string()
                            }
                        }
                        'X' => {
                            if let Some(i) = raw.as_i64() {
                                format!("{:X}", i)
                            } else {
                                "0".to_string()
                            }
                        }
                        'c' => {
                            if let Some(i) = raw.as_i64() {
                                char::from_u32(i as u32).map(|c| c.to_string()).unwrap_or_default()
                            } else {
                                raw.str().chars().next().map(|c| c.to_string()).unwrap_or_default()
                            }
                        }
                        _ => unreachable!(),
                    };

                    // Apply width
                    let padded = if let Some(w) = width {
                        if flags.contains('0') {
                            format!("{:0>width$}", formatted, width = w)
                        } else {
                            format!("{:>width$}", formatted, width = w)
                        }
                    } else {
                        formatted
                    };
                    result.push_str(&padded);
                }
                Some(c) => return Err(format!("unsupported format character '{}'", c)),
            }
        } else {
            result.push(ch);
        }
    }

    Ok(result)
}

/// printf-style byte-string interpolation (`bytes`/`bytearray`'s `%` operator, PEP 461).
/// Deliberately covers the common conversions (`%s`/`%b`/`%d`/`%i`/`%o`/`%x`/`%X`/`%c`/`%%`
/// plus width) rather than CPython's full adversarial surface (`%a`'s exact unicode-escaping
/// repr, the `-` left-justify flag) — those are not exercised outside `test_format.py`'s own
/// exhaustive self-test.
pub(crate) fn bytes_interpolate(fmt: &[u8], arg: &PyObjectRef) -> Result<Vec<u8>, String> {
    let mut result: Vec<u8> = Vec::new();
    let mut i = 0;

    let mut arg_iter: Option<Box<dyn Iterator<Item = PyObjectRef>>> = None;
    {
        let obj = arg.borrow();
        if let PyObject::Tuple(items) = &*obj {
            arg_iter = Some(Box::new(items.clone().into_iter()));
        }
    }
    let arg0 = arg.clone();
    let mut get_arg = || -> PyObjectRef {
        if let Some(ref mut it) = arg_iter {
            it.next().unwrap_or_else(py_none)
        } else {
            arg0.clone()
        }
    };

    // Bytes-like: bytes, bytearray, or memoryview-over-bytes — used by %s/%b.
    let as_bytes_like = |v: &PyObjectRef| -> Option<Vec<u8>> {
        match &*v.borrow() {
            PyObject::Bytes(b) => Some(b.clone()),
            PyObject::ByteArray(b) => Some(b.clone()),
            _ => None,
        }
    };

    while i < fmt.len() {
        let ch = fmt[i];
        i += 1;
        if ch != b'%' {
            result.push(ch);
            continue;
        }
        let mut flags_zero = false;
        if i < fmt.len() && fmt[i] == b'0' {
            flags_zero = true;
            i += 1;
        }
        let mut width_str = String::new();
        while i < fmt.len() && fmt[i].is_ascii_digit() {
            width_str.push(fmt[i] as char);
            i += 1;
        }
        let width: Option<usize> = if width_str.is_empty() { None } else {
            Some(width_str.parse().map_err(|_| "invalid width".to_string())?)
        };
        if i >= fmt.len() { return Err("incomplete format".to_string()); }
        let conv = fmt[i];
        i += 1;
        let formatted: Vec<u8> = match conv {
            b'%' => vec![b'%'],
            b's' | b'b' => {
                let raw = get_arg();
                as_bytes_like(&raw).ok_or_else(|| format!(
                    "%{} requires a bytes-like object, or an object that implements __bytes__, not '{}'",
                    conv as char, raw.borrow().type_name()
                ))?
            }
            b'r' => {
                let raw = get_arg();
                let s = raw.repr();
                if !s.is_ascii() { return Err("%r result contains non-ASCII data".to_string()); }
                s.into_bytes()
            }
            b'd' | b'i' | b'o' | b'x' | b'X' => {
                let raw = get_arg();
                let n = raw.as_i64().unwrap_or(0);
                (match conv {
                    b'd' | b'i' => n.to_string(),
                    b'o' => format!("{:o}", n),
                    b'x' => format!("{:x}", n),
                    b'X' => format!("{:X}", n),
                    _ => unreachable!(),
                }).into_bytes()
            }
            b'c' => {
                let raw = get_arg();
                if let Some(n) = raw.as_i64() {
                    vec![n as u8]
                } else if let Some(b) = as_bytes_like(&raw) {
                    if b.len() != 1 { return Err("%c requires an integer in range(256) or a single byte".to_string()); }
                    b
                } else {
                    return Err("%c requires an integer in range(256) or a single byte".to_string());
                }
            }
            c => return Err(format!("unsupported format character '{}'", c as char)),
        };
        let padded = if let Some(w) = width {
            if formatted.len() >= w {
                formatted
            } else {
                let pad_len = w - formatted.len();
                let pad_byte = if flags_zero { b'0' } else { b' ' };
                let mut v = vec![pad_byte; pad_len];
                v.extend(formatted);
                v
            }
        } else {
            formatted
        };
        result.extend(padded);
    }

    Ok(result)
}
