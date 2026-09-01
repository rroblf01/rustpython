// Split out of the former monolithic object.rs (see the file-splitting
// refactor's memory entry for context) — this file holds the VM-pointer
// thread-locals, the class registry backing `type.__subclasses__()`, the
// `py_*` constructor helpers, and the `%`-operator string interpolation
// implementation.
use super::*;

mod string;
pub(crate) use string::string_interpolate;
mod bytes;
pub(crate) use bytes::bytes_interpolate;

thread_local! {
    /// The active VirtualMachine for this thread. `Cell` (not RefCell) so
    /// save/restore around nested executes never conflicts; raw pointers are
    /// Copy, which Cell::get/set require.
    pub static VM_PTR: std::cell::Cell<Option<*mut crate::vm::VirtualMachine>> = const { std::cell::Cell::new(None) };
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
    // Save/restore via Cell::get/set (no held borrow): nested with_vm_mut /
    // execute() calls during `f` see the SAME active VM, and finalizer code
    // that pins its own pointer cannot hit a RefCell conflict.
    let ptr = VM_PTR.get();
    if let Some(ptr) = ptr {
        // SAFETY: see historical note — set by execute() while running.
        let vm = unsafe { &mut *ptr };
        let out = f(vm);
        VM_PTR.set(Some(ptr));
        Ok(out)
    } else {
        Err(PyError::runtime_error("no active VM"))
    }
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
            if let PyObject::Type {
                name: tname, dict, ..
            } = &*cls.borrow()
            {
                if tname != name {
                    continue;
                }
                let module_name = dict
                    .get_str("__module__")
                    .map(|m| m.str())
                    .unwrap_or_default();
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
    NOT_IMPLEMENTED_SINGLETON.with(|c| {
        c.borrow()
            .clone()
            .expect("NotImplemented singleton not yet seeded")
    })
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
        _ => {
            let type_name = borrowed.type_name().to_string();
            drop(borrowed);
            // Fallback: any iterable (dict views, generators, etc.)
            if let Ok(iterator) = crate::object::builtin_iter(&[obj.clone()]) {
                let mut elts: Vec<PyObjectRef> = Vec::new();
                loop {
                    match crate::object::builtin_next(&[iterator.clone()]) {
                        Ok(v) => elts.push(v),
                        Err(PyError::StopIteration) => break,
                        Err(PyError::Exception(msg, _)) if msg == "StopIteration" => break,
                        Err(e) if crate::object::is_stop_iteration_error(&e) => break,
                        Err(e) => return Err(e),
                    }
                }
                return PySet::from_vec(elts);
            }
            Err(PyError::type_error(format!(
                "cannot convert '{}' to set",
                type_name
            )))
        }
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
/// Fixed-point formatting that tolerates huge precisions (Rust's own
/// `format!("{:.prec$}", ...)` panics past a limit). f64 decimal expansions
/// are exact within ~55 digits, so format at min(prec, 100) and pad with
/// zeros to `prec`.
pub(crate) fn format_fixed_padded(f: f64, prec: usize) -> String {
    if f.is_nan() {
        return "nan".to_string();
    }
    if f.is_infinite() {
        return if f.is_sign_negative() {
            "-inf".to_string()
        } else {
            "inf".to_string()
        };
    }
    if prec <= 100 {
        return format!("{:.prec$}", f, prec = prec);
    }
    let base = format!("{:.prec$}", f, prec = 100);
    // Strip to the integer part + existing decimals, then pad.
    let mut s = base;
    if let Some(dot) = s.find('.') {
        let decimals = s.len() - dot - 1;
        if decimals < prec {
            s.push_str(&"0".repeat(prec - decimals));
        }
    } else {
        s.push('.');
        s.push_str(&"0".repeat(prec));
    }
    s
}

/// Apply the `%+` / `% ` sign flags to a float-formatted string (a leading
/// `+`/space for non-negative values; nans are always "positive" so they get
/// the sign like CPython).
pub(crate) fn apply_sign_flag(s: String, flags: &str) -> String {
    if !s.starts_with('-') && flags.contains('+') {
        format!("+{}", s)
    } else if !s.starts_with('-') && flags.contains(' ') {
        format!(" {}", s)
    } else {
        s
    }
}

/// `%e`/`%E` scientific-notation formatting: mantissa with `prec` digits
/// after the point, then `e±NN` (exponent at least 2 digits, `E` for %E).
/// `alternate` (#) keeps the decimal point when prec == 0.
pub(crate) fn format_percent_e(f: f64, prec: usize, alternate: bool, upper: bool) -> String {
    // nans are always considered positive when formatted (CPython prints
    // 'nan' even for a -nan whose sign bit is set).
    let sign = if f.is_nan() {
        ""
    } else if f.is_sign_negative() {
        "-"
    } else {
        ""
    };
    let abs = f.abs();
    if abs == 0.0 {
        let mut s = format!("{:.*}e+00", prec, 0.0f64);
        if alternate && prec == 0 {
            s = format!("{}.e+00", 0.0f64);
        }
        let e_char = if upper { 'E' } else { 'e' };
        return format!("{}{}", sign, s.replace('e', &e_char.to_string()));
    }
    // Get the mantissa/exponent from {:e} (exact-ish decimal), then round
    // the mantissa to `prec` decimals — the old `abs / 10^exp` division
    // introduced float error (1.230005 -> 1.23001).
    let sci = format!("{:e}", abs);
    let (mant_s, exp_s) = match sci.split_once('e') {
        Some(p) => p,
        // inf/nan have no 'e' in {:e} output — Rust prints "NaN"/"inf",
        // CPython prints "nan"/"inf" for %e (and "NAN"/"INF" for %E).
        None => {
            let base = sci.to_ascii_lowercase();
            let base = if upper {
                base.to_ascii_uppercase()
            } else {
                base
            };
            return format!("{}{}", sign, base);
        }
    };
    let exp: i32 = exp_s.parse().unwrap_or(0);
    // Round the mantissa's DIGITS to `prec` decimals with round-half-even on
    // the exact decimal string (format!("{:.p}", f64) uses the nearest double
    // and rounds 1.230005 up -> 1.23001; CPython's exact half-even gives
    // 1.23000). mant_s is the shortest repr like "1.230005".
    let digits: Vec<char> = mant_s.chars().filter(|&c| c != '.').collect();
    let keep = prec + 1; // significant digits (1 before the point)
    let mut out_digits: Vec<char> = digits.clone();
    let mut exp_out = exp;
    if digits.len() > keep {
        // round to `keep` digits
        let mut rounded: Vec<char> = digits[..keep].to_vec();
        let next = digits[keep];
        let round_up = if next > '5' {
            true
        } else if next < '5' {
            false
        } else {
            // next == '5': round half to even on the last kept digit
            let has_nonzero = digits[keep + 1..].iter().any(|&c| c != '0');
            if has_nonzero {
                true
            } else {
                let last = rounded[keep - 1];
                last.to_digit(10).unwrap_or(0) % 2 == 1
            }
        };
        if round_up {
            // increment the digit string (with carry)
            let mut i = keep as isize - 1;
            loop {
                if i < 0 {
                    rounded.insert(0, '1');
                    exp_out += 1;
                    break;
                }
                let d = rounded[i as usize].to_digit(10).unwrap_or(0);
                if d == 9 {
                    rounded[i as usize] = '0';
                    i -= 1;
                } else {
                    rounded[i as usize] = char::from_digit(d + 1, 10).unwrap_or('0');
                    break;
                }
            }
            // a carry out of the top digit ("999" -> "1000") yields one extra
            // digit — keep just `keep` of them (1.00e+04, not 1.000e+04)
            rounded.truncate(keep);
        }
        out_digits = rounded;
    }
    while out_digits.len() < keep {
        out_digits.push('0');
    }
    // build "d.ddd" from out_digits (drop trailing zeros for prec>0 handled by caller)
    let int_c = out_digits[0];
    let s = if prec == 0 {
        if alternate {
            format!("{}.", int_c)
        } else {
            int_c.to_string()
        }
    } else {
        format!("{}.{}", int_c, out_digits[1..].iter().collect::<String>())
    };
    let e_char = if upper { 'E' } else { 'e' };
    let exp_sign = if exp_out < 0 { '-' } else { '+' };
    format!("{}{}{}{}{:02}", sign, s, e_char, exp_sign, exp_out.abs())
}

/// `%g`/`%G` general formatting: precision = significant digits; uses
/// scientific if the exponent is < -4 or >= precision, else fixed; strips
/// trailing zeros unless `#`. With `add_dot_0` (CPython's ADD_DOT_0, used by
/// the EMPTY float presentation type with a precision) the scientific
/// threshold shifts to `exp >= precision-1` so 1234.56 with precision 4
/// renders as '1.235e+03' rather than '1235' (and the fixed form always
/// keeps a fractional digit).
pub(crate) fn format_percent_g(f: f64, prec: usize, alternate: bool, add_dot_0: bool) -> String {
    let abs = f.abs();
    if abs.is_nan() {
        return "nan".to_string();
    }
    if abs.is_infinite() {
        return if f.is_sign_negative() {
            "-inf".to_string()
        } else {
            "inf".to_string()
        };
    }
    if abs == 0.0 {
        let sign = if f.is_sign_negative() { "-" } else { "" };
        let mut s = format!("{}{:.*}", sign, prec.saturating_sub(1).max(0), 0.0f64);
        if alternate {
            if !s.contains('.') {
                s.push('.');
            }
            return s;
        }
        while s.contains('.') && s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
        return s;
    }
    let exp = abs.log10().floor() as i32;
    // Precision 0 is treated as 1 significant digit for the sci threshold
    // (CPython: %.0g of 1 is '1', but %.0g of 10 is '1e+01').
    let eff_prec = prec.max(1);
    let sci_threshold = if add_dot_0 {
        eff_prec as i32 - 1
    } else {
        eff_prec as i32
    };
    let use_sci = exp < -4 || exp >= sci_threshold;
    if use_sci {
        // %g scientific uses precision-1 decimals
        let mut s = format_percent_e(f, prec.saturating_sub(1), alternate, false);
        // strip trailing zeros in mantissa unless alternate
        if !alternate {
            let e_pos = s.find('e').unwrap_or(s.len());
            let mut mant = s[..e_pos].to_string();
            if mant.contains('.') {
                while mant.ends_with('0') {
                    mant.pop();
                }
                if mant.ends_with('.') {
                    mant.pop();
                }
            }
            s = format!("{}{}", mant, &s[e_pos..]);
        }
        s
    } else {
        // fixed: prec = significant digits = prec-1 decimals
        let decimals = (eff_prec as i32 - 1 - exp).max(0) as usize;
        let mut s = format!("{:.*}", decimals, f);
        if !alternate {
            if s.contains('.') {
                while s.ends_with('0') {
                    s.pop();
                }
                if s.ends_with('.') {
                    s.pop();
                }
            }
        } else if !s.contains('.') {
            s.push('.');
        }
        s
    }
}

/// Zero-pad an integer conversion (`%x`/`%X`/`%o`/`%d`) to `precision`
/// digits, accounting for a `0x`/`0X`/`0o` alternate prefix.
/// Extract the BigInt value of an object for integer %-conversions.
pub(crate) fn bigint_of(raw: &PyObjectRef) -> num_bigint::BigInt {
    let b = raw.borrow();
    match &*b {
        PyObject::Int(bi) => bi.clone(),
        PyObject::Bool(bb) => num_bigint::BigInt::from(if *bb { 1i32 } else { 0 }),
        // A float truncates toward zero for %d/%i ('%d' % 3.14 -> 3); the
        // exact integer of a whole f64 is preserved.
        PyObject::Float(f) if f.is_finite() => num_bigint::BigInt::from(*f as i128),
        _ => num_bigint::BigInt::from(raw.as_i64().unwrap_or(0)),
    }
}

pub(crate) fn zero_pad_precision(mut s: String, precision: usize, has_prefix: bool) -> String {
    if precision > 1000 {
        return s;
    }
    // Strip a leading sign first so it stays before the 0x prefix and zeros
    // ('%#.23x' of -big -> '-0x0012...', not '-00x12...').
    let (sign, rest) = if let Some(r) = s.strip_prefix('-') {
        ("-", r.to_string())
    } else if let Some(r) = s.strip_prefix('+') {
        ("+", r.to_string())
    } else if let Some(r) = s.strip_prefix(' ') {
        (" ", r.to_string())
    } else {
        ("", s.clone())
    };
    let prefix_len = if has_prefix { 2 } else { 0 };
    let body_len = rest.len().saturating_sub(prefix_len);
    if precision > body_len {
        let zeros = "0".repeat(precision - body_len);
        if has_prefix {
            let (prefix, body) = rest.split_at(2);
            format!("{}{}{}{}", sign, prefix, zeros, body)
        } else {
            format!("{}{}{}", sign, zeros, rest)
        }
    } else {
        s
    }
}

/// Byte index of the current position in the format string (for error
/// messages like "unsupported format character 'x' (0x78) at index N").
pub(crate) fn byte_index_in(fmt: &str, rest: &str) -> usize {
    fmt.len().saturating_sub(rest.len())
}

