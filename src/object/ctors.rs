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
/// Fixed-point formatting that tolerates huge precisions (Rust's own
/// `format!("{:.prec$}", ...)` panics past a limit). f64 decimal expansions
/// are exact within ~55 digits, so format at min(prec, 100) and pad with
/// zeros to `prec`.
fn format_fixed_padded(f: f64, prec: usize) -> String {
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

/// `%e`/`%E` scientific-notation formatting: mantissa with `prec` digits
/// after the point, then `e±NN` (exponent at least 2 digits, `E` for %E).
/// `alternate` (#) keeps the decimal point when prec == 0.
pub(crate) fn format_percent_e(f: f64, prec: usize, alternate: bool, upper: bool) -> String {
    let sign = if f.is_sign_negative() { "-" } else { "" };
    let abs = f.abs();
    if abs == 0.0 {
        let mut s = format!("{:.*}e+00", prec, 0.0f64);
        if alternate && prec == 0 { s = format!("{}.e+00", 0.0f64); }
        let e_char = if upper { 'E' } else { 'e' };
        return format!("{}{}", sign, s.replace('e', &e_char.to_string()));
    }
    // Get the mantissa/exponent from {:e} (exact-ish decimal), then round
    // the mantissa to `prec` decimals — the old `abs / 10^exp` division
    // introduced float error (1.230005 -> 1.23001).
    let sci = format!("{:e}", abs);
    let (mant_s, exp_s) = match sci.split_once('e') {
        Some(p) => p,
        // inf/nan have no 'e' in {:e} output
        None => return format!("{}{}", sign, sci),
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
/// trailing zeros unless `#`.
pub(crate) fn format_percent_g(f: f64, prec: usize, alternate: bool) -> String {
    let abs = f.abs();
    if abs == 0.0 {
        let sign = if f.is_sign_negative() { "-" } else { "" };
        let mut s = format!("{}{:.*}", sign, prec.saturating_sub(1).max(0), 0.0f64);
        if alternate {
            if !s.contains('.') {
                s.push('.');
            }
            return s;
        }
        while s.contains('.') && s.ends_with('0') { s.pop(); }
        if s.ends_with('.') { s.pop(); }
        return s;
    }
    let exp = abs.log10().floor() as i32;
    // Precision 0 is treated as 1 significant digit for the sci threshold
    // (CPython: %.0g of 1 is '1', but %.0g of 10 is '1e+01').
    let eff_prec = prec.max(1);
    let use_sci = exp < -4 || exp >= eff_prec as i32;
    if use_sci {
        // %g scientific uses precision-1 decimals
        let mut s = format_percent_e(f, prec.saturating_sub(1), alternate, false);
        // strip trailing zeros in mantissa unless alternate
        if !alternate {
            let e_pos = s.find('e').unwrap_or(s.len());
            let mut mant = s[..e_pos].to_string();
            if mant.contains('.') {
                while mant.ends_with('0') { mant.pop(); }
                if mant.ends_with('.') { mant.pop(); }
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
                while s.ends_with('0') { s.pop(); }
                if s.ends_with('.') { s.pop(); }
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
fn bigint_of(raw: &PyObjectRef) -> num_bigint::BigInt {
    let b = raw.borrow();
    match &*b {
        PyObject::Int(bi) => bi.clone(),
        PyObject::Bool(bb) => num_bigint::BigInt::from(if *bb { 1i32 } else { 0 }),
        // A float truncates toward zero for %d/%i ('%d' % 3.14 -> 3); the
        // exact integer of a whole f64 is preserved.
        PyObject::Float(f) if f.is_finite() => {
            num_bigint::BigInt::from(*f as i128)
        }
        _ => num_bigint::BigInt::from(raw.as_i64().unwrap_or(0)),
    }
}

fn zero_pad_precision(mut s: String, precision: usize, has_prefix: bool) -> String {
    if precision > 1000 { return s; }
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
fn byte_index_in(fmt: &str, rest: &str) -> usize {
    fmt.len().saturating_sub(rest.len())
}

pub(crate) fn string_interpolate(fmt: &str, arg: &PyObjectRef) -> Result<String, String> {    let mut result = String::new();
    let mut chars = fmt.chars();
    let mut converted = 0usize;

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
            // Parse flags (`0`, `-`, `+`, space, `#`) — previously only `0`.
            let mut flags = String::new();
            loop {
                let mut peek = chars.clone();
                match peek.next() {
                    Some(c) if c == '0' || c == '-' || c == '+' || c == ' ' || c == '#' => {
                        flags.push(c);
                        chars.next();
                    }
                    _ => break,
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
                // Dynamic precision `.*` — the value comes from the next arg.
                if chars.clone().next() == Some('*') {
                    chars.next();
                    let p = get_arg();
                    precision = Some(p.as_i64().unwrap_or(6).max(0) as usize);
                } else {
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
            }

            let had_spec = !flags.is_empty() || width.is_some() || precision.is_some() || mapping_key.is_some();
            match chars.next() {
                None => return Err("incomplete format".to_string()),
                // `%%` is only an escape when the % follows immediately — a
                // % after a flag/width/precision is an unsupported character
                // ('% %s' reports the second % at index 2).
                Some('%') if had_spec => {
                    return Err(format!("unsupported format character '%' (0x25) at index {}",
                        byte_index_in(fmt, chars.as_str())));
                }
                Some('%') => result.push('%'),
                Some(conv @ 's') | Some(conv @ 'r') | Some(conv @ 'f') | Some(conv @ 'd') | Some(conv @ 'i')
                | Some(conv @ 'o') | Some(conv @ 'x') | Some(conv @ 'X') | Some(conv @ 'c')
                | Some(conv @ 'e') | Some(conv @ 'E') | Some(conv @ 'g') | Some(conv @ 'G') | Some(conv @ 'u') | Some(conv @ 'F') | Some(conv @ 'a') => {
                    converted += 1;
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

                    if matches!(conv, 'f' | 'F' | 'e' | 'E' | 'g' | 'G')
                        && !matches!(&*raw.borrow(), PyObject::Int(_) | PyObject::Bool(_) | PyObject::Float(_)) {
                        return Err(format!("must be real number, not {}", raw.borrow().type_name()));
                    }
                    let formatted = match conv {
                        's' => raw.str(),
                        'r' => raw.repr(),
                        'a' => {
                            // ascii() repr: repr with ALL non-ASCII escaped
                            let r = raw.repr();
                            let mut out = String::new();
                            for c in r.chars() {
                                if c.is_ascii() { out.push(c); }
                                else if (c as u32) <= 0xFFFF { out.push_str(&format!("\\u{:04x}", c as u32)); }
                                else { out.push_str(&format!("\\U{:08x}", c as u32)); }
                            }
                            out
                        }
                        'f' => {
                            if !matches!(&*raw.borrow(), PyObject::Int(_) | PyObject::Bool(_) | PyObject::Float(_)) {
                                return Err(format!("must be real number, not {}", raw.borrow().type_name()));
                            }
                            let f = raw.as_f64().unwrap_or(0.0);
                            let prec = precision.unwrap_or(6);
                            // CPython's test_format exercises %12.*f with
                            // precision 123456 (must work); sys.maxsize must
                            // still overflow. Rust's own format! panics at
                            // large precisions, so format at <=100 decimals
                            // (exact for any f64) and pad the rest with
                            // zeros.
                            if prec > 200000 { return Err("precision too big".to_string()); }
                            let mut s = format_fixed_padded(f, prec);
                            if flags.contains('#') && !s.contains('.') {
                                s.push('.');
                            }
                            s
                        }
                        'F' => {
                            let f = raw.as_f64().unwrap_or(0.0);
                            let prec = precision.unwrap_or(6);
                            if prec > 200000 { return Err("precision too big".to_string()); }
                            let mut s = format_fixed_padded(f, prec);
                            if flags.contains('#') && !s.contains('.') {
                                s.push('.');
                            }
                            s
                        }
                        'e' | 'E' => {
                            let f = raw.as_f64().unwrap_or(0.0);
                            let prec = precision.unwrap_or(6);
                            format_percent_e(f, prec, flags.contains('#'), false)
                        }
                        'g' | 'G' => {
                            let f = raw.as_f64().unwrap_or(0.0);
                            let prec = precision.unwrap_or(6);
                            format_percent_g(f, prec, flags.contains('#'))
                        }
                        'd' | 'i' | 'u' => {
                            // A non-numeric arg must raise ("%d format: a
                            // real number is required, not str").
                            if !matches!(&*raw.borrow(), PyObject::Int(_) | PyObject::Bool(_) | PyObject::Float(_)) {
                                return Err(format!("%{} format: a real number is required, not {}", conv, raw.borrow().type_name()));
                            }
                            // Handle big ints that overflow i64, and float
                            // whole numbers ('%d' % -1.2e29) — stringify via
                            // BigInt (test_common_format).
                            let mut s = bigint_of(&raw).to_string();
                            if !s.starts_with('-') {
                                if flags.contains('+') { s = format!("+{}", s); }
                                else if flags.contains(' ') { s = format!(" {}", s); }
                            }
                            // `.precision` zero-pads %d/%i (`%.100d` of 1
                            // -> 99 zeros then 1). A huge precision
                            // (e.g. sys.maxsize from `%.*d`) must raise,
                            // not allocate an astronomical string.
                            if let Some(p) = precision {
                                if p > 1000 { return Err("precision too big".to_string()); }
                                if p > s.len() {
                                    format!("{}{}", "0".repeat(p - s.len()), s)
                                } else { s }
                            } else { s }
                        }
                        'o' => {
                            if !matches!(&*raw.borrow(), PyObject::Int(_) | PyObject::Bool(_)) {
                                return Err(format!("%{} format: an integer is required, not {}", conv, raw.borrow().type_name()));
                            }
                            let bi = bigint_of(&raw);
                            let neg = bi.sign() == num_bigint::Sign::Minus;
                            let mut s = if flags.contains('#') {
                                if neg { format!("-0o{:o}", bi.abs()) } else { format!("0o{:o}", bi) }
                            } else { format!("{:o}", bi) };
                            if !s.starts_with('-') && flags.contains('+') { s = format!("+{}", s); }
                            else if !s.starts_with('-') && flags.contains(' ') { s = format!(" {}", s); }
                            if let Some(p) = precision { s = zero_pad_precision(s, p, flags.contains('#')) }
                            s
                        }
                        'x' => {
                            if !matches!(&*raw.borrow(), PyObject::Int(_) | PyObject::Bool(_)) {
                                return Err(format!("%{} format: an integer is required, not {}", conv, raw.borrow().type_name()));
                            }
                            let bi = bigint_of(&raw);
                            let neg = bi.sign() == num_bigint::Sign::Minus;
                            let mut s = if flags.contains('#') {
                                if neg { format!("-0x{:x}", bi.abs()) } else { format!("0x{:x}", bi) }
                            } else { format!("{:x}", bi) };
                            if !s.starts_with('-') && flags.contains('+') { s = format!("+{}", s); }
                            else if !s.starts_with('-') && flags.contains(' ') { s = format!(" {}", s); }
                            if let Some(p) = precision { s = zero_pad_precision(s, p, flags.contains('#')) }
                            s
                        }
                        'X' => {
                            if !matches!(&*raw.borrow(), PyObject::Int(_) | PyObject::Bool(_)) {
                                return Err(format!("%{} format: an integer is required, not {}", conv, raw.borrow().type_name()));
                            }
                            let bi = bigint_of(&raw);
                            let neg = bi.sign() == num_bigint::Sign::Minus;
                            let mut s = if flags.contains('#') {
                                if neg { format!("-0X{:X}", bi.abs()) } else { format!("0X{:X}", bi) }
                            } else { format!("{:X}", bi) };
                            if !s.starts_with('-') && flags.contains('+') { s = format!("+{}", s); }
                            else if !s.starts_with('-') && flags.contains(' ') { s = format!(" {}", s); }
                            if let Some(p) = precision { s = zero_pad_precision(s, p, flags.contains('#')) }
                            s
                        }
                        'c' => {
                            if let Some(i) = raw.as_i64() {
                                // %c of an int must be a valid Unicode scalar
                                // (0..0x110000) — out of range raises
                                // OverflowError ("%c arg not in range(0x110000)").
                                match char::from_u32(i as u32) {
                                    Some(c) => c.to_string(),
                                    None => return Err("%c arg not in range(0x110000) [overflow]".to_string()),
                                }
                            } else if matches!(&*raw.borrow(), PyObject::Str(_)) {
                                let s = raw.str();
                                if s.chars().count() != 1 {
                                    return Err(format!(
                                        "%c requires an int or a unicode character, not a string of length {}",
                                        s.chars().count()
                                    ));
                                }
                                s.chars().next().map(|c| c.to_string()).unwrap_or_default()
                            } else {
                                return Err(format!(
                                    "%c requires an int or a unicode character, not {}",
                                    raw.borrow().type_name()
                                ));
                            }
                        }
                        _ => unreachable!(),
                    };

                    // Apply width (respecting - left-justify, 0 zero-pad)
                    let padded = if let Some(w) = width {
                        if flags.contains('-') {
                            format!("{:<width$}", formatted, width = w)
                        } else if flags.contains('0') {
                            // Zero-pad AFTER the sign and any 0x/0o prefix
                            // (`%032d` of -big -> '-000...digits';
                            // `%#027x` -> '0x00001234567890abcdef12345').
                            let (sign, rest) = if let Some(r) = formatted.strip_prefix('-') {
                                ("-", r)
                            } else if let Some(r) = formatted.strip_prefix('+') {
                                ("+", r)
                            } else if let Some(r) = formatted.strip_prefix(' ') {
                                (" ", r)
                            } else {
                                ("", formatted.as_str())
                            };
                            let (prefix, body) = if let Some(r) = rest.strip_prefix("0x") {
                                ("0x", r)
                            } else if let Some(r) = rest.strip_prefix("0X") {
                                ("0X", r)
                            } else if let Some(r) = rest.strip_prefix("0o") {
                                ("0o", r)
                            } else {
                                ("", rest)
                            };
                            let padded = format!("{:0>width$}", body, width = w.saturating_sub(sign.len() + prefix.len()));
                            format!("{}{}{}", sign, prefix, padded)
                        } else {
                            format!("{:>width$}", formatted, width = w)
                        }
                    } else {
                        formatted
                    };
                    result.push_str(&padded);
                }
                Some(c) => return Err(format!("unsupported format character '{}' (0x{:02x}) at index {}",
                    c, c as u32, byte_index_in(fmt, chars.as_str()))),
            }
        } else {
            result.push(ch);
        }
    }

    // Real CPython raises if a single non-mapping arg is provided but no
    // conversion consumed it, or a tuple has MORE elements than specs.
    let arg_is_dict = matches!(&*arg0.borrow(), PyObject::Dict(_));
    if arg_iter.is_none() && !arg_is_dict && converted == 0 {
        return Err("not all arguments converted during string formatting".to_string());
    }
    if let Some(it) = arg_iter {
        let mut it = it;
        if it.next().is_some() {
            return Err("not all arguments converted during string formatting".to_string());
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
            PyObject::MemoryView { .. } => crate::object::mv_tobytes(v).ok(),
            _ => None,
        }
    };

    let mut converted = 0usize;
    while i < fmt.len() {
        let ch = fmt[i];
        i += 1;
        if ch != b'%' {
            result.push(ch);
            continue;
        }
        let mut flags_zero = false;
        let mut flags_alt = false;
        let mut flags_minus = false;
        let mut flags_plus = false;
        let mut flags_space = false;
        loop {
            if i >= fmt.len() { break; }
            match fmt[i] {
                b'0' => { flags_zero = true; i += 1; }
                b'#' => { flags_alt = true; i += 1; }
                b'-' => { flags_minus = true; i += 1; }
                b'+' => { flags_plus = true; i += 1; }
                b' ' => { flags_space = true; i += 1; }
                _ => break,
            }
        }
        let mut width_str = String::new();
        while i < fmt.len() && fmt[i].is_ascii_digit() {
            width_str.push(fmt[i] as char);
            i += 1;
        }
        let width: Option<usize> = if width_str.is_empty() { None } else {
            Some(width_str.parse().map_err(|_| "invalid width".to_string())?)
        };
        // Parse optional `.precision` (e.g. b"%.2f", b"%.0d") and dynamic `.*`.
        let mut precision: Option<usize> = None;
        if i < fmt.len() && fmt[i] == b'.' {
            i += 1;
            if i < fmt.len() && fmt[i] == b'*' {
                i += 1;
                let p = get_arg();
                precision = Some(p.as_i64().unwrap_or(6).max(0) as usize);
            } else {
                let mut prec_str = String::new();
                while i < fmt.len() && fmt[i].is_ascii_digit() {
                    prec_str.push(fmt[i] as char);
                    i += 1;
                }
                precision = Some(if prec_str.is_empty() { 0 } else {
                    prec_str.parse().map_err(|_| "invalid precision".to_string())?
                });
            }
        }
        if i >= fmt.len() { return Err("incomplete format".to_string()); }
        let conv = fmt[i];
        i += 1;
        // `%%` is only an escape when the % follows immediately — after a
        // flag it is an unsupported character ('% %s' -> the second %).
        let had_spec = flags_zero || flags_alt || flags_minus || flags_plus || flags_space || width.is_some() || precision.is_some();
        if conv == b'%' && had_spec {
            return Err(format!("unsupported format character '%' (0x25) at index {}", i - 1));
        }
        if conv != b'%' { converted += 1; }
        let formatted: Vec<u8> = match conv {
            b'%' => vec![b'%'],
            b's' | b'b' => {
                let raw = get_arg();
                if let Some(b) = as_bytes_like(&raw) {
                    b
                } else if let Some(f) = raw.borrow().get_attribute("__bytes__").ok() {
                    crate::object::call_bound_method(f, raw.clone(), vec![])
                        .and_then(|r| {
                            let rb = r.borrow();
                            if let PyObject::Bytes(b) = &*rb { Ok(b.clone()) } else { Err(crate::object::PyError::type_error("__bytes__ returned non-bytes")) }
                        })
                        .map_err(|_| format!(
                            "%{} requires a bytes-like object, or an object that implements __bytes__, not '{}'",
                            conv as char, raw.borrow().type_name()
                        ))?
                } else {
                    return Err(format!(
                        "%{} requires a bytes-like object, or an object that implements __bytes__, not '{}'",
                        conv as char, raw.borrow().type_name()
                    ));
                }
            }
            b'r' => {
                let raw = get_arg();
                let s = raw.repr();
                // Escape non-ASCII like %a (CPython: b'%r' % 'Մ' ==
                // b"'\\u0544'").
                let mut bytes: Vec<u8> = Vec::new();
                for ch in s.chars() {
                    if ch.is_ascii() {
                        bytes.push(ch as u8);
                    } else if (ch as u32) <= 0xFFFF {
                        bytes.extend_from_slice(format!("\\u{:04x}", ch as u32).as_bytes());
                    } else {
                        bytes.extend_from_slice(format!("\\U{:08x}", ch as u32).as_bytes());
                    }
                }
                bytes
            }
            b'a' => {
                // ascii() representation: ASCII chars verbatim, non-ASCII
                // as \uXXXX / \UXXXXXXXX escapes (CPython: b'%a' % 'Մ'
                // == b"'\\u0544'").
                let raw = get_arg();
                let s = raw.repr();
                let mut bytes: Vec<u8> = Vec::new();
                for ch in s.chars() {
                    if ch.is_ascii() {
                        bytes.push(ch as u8);
                    } else if (ch as u32) <= 0xFFFF {
                        bytes.extend_from_slice(format!("\\u{:04x}", ch as u32).as_bytes());
                    } else {
                        bytes.extend_from_slice(format!("\\U{:08x}", ch as u32).as_bytes());
                    }
                }
                bytes
            }
            b'd' | b'i' | b'u' | b'o' | b'x' | b'X' => {
                let raw = get_arg();
                let is_real = matches!(conv, b'd' | b'i' | b'u');
                let allowed = if is_real {
                    matches!(&*raw.borrow(), PyObject::Int(_) | PyObject::Bool(_) | PyObject::Float(_))
                } else {
                    // %x/%X/%o reject even whole floats (an integer is required)
                    matches!(&*raw.borrow(), PyObject::Int(_) | PyObject::Bool(_))
                };
                if !allowed {
                    return Err(format!(
                        "%{} format: {} is required, not {}", conv as char,
                        if is_real { "a real number" } else { "an integer" },
                        raw.borrow().type_name()
                    ));
                }
                let bi = bigint_of(&raw);
                let mut s = match conv {
                    b'd' | b'i' | b'u' => {
                        if let Some(p) = precision {
                            // zero-pad to precision digits (`%.100d` of 1)
                            if p > 1000 { return Err("precision too big".to_string()); }
                            let s2 = bi.to_string();
                            if p > s2.len() {
                                format!("{}{}", "0".repeat(p - s2.len()), s2)
                            } else { s2 }
                        } else { bi.to_string() }
                    }
                    b'o' => if flags_alt {
                        if bi.sign() == num_bigint::Sign::Minus { format!("-0o{:o}", bi.abs()) } else { format!("0o{:o}", bi) }
                    } else { format!("{:o}", bi) },
                    b'x' => if flags_alt {
                        if bi.sign() == num_bigint::Sign::Minus { format!("-0x{:x}", bi.abs()) } else { format!("0x{:x}", bi) }
                    } else { format!("{:x}", bi) },
                    b'X' => if flags_alt {
                        if bi.sign() == num_bigint::Sign::Minus { format!("-0X{:X}", bi.abs()) } else { format!("0X{:X}", bi) }
                    } else { format!("{:X}", bi) },
                    _ => unreachable!(),
                };
                if !s.starts_with('-') {
                    if flags_plus { s = format!("+{}", s); }
                    else if flags_space { s = format!(" {}", s); }
                }
                if let Some(p) = precision {
                    s = zero_pad_precision(s, p, flags_alt);
                }
                if let Some(w) = width {
                    s = if flags_minus {
                        format!("{:<width$}", s, width = w)
                    } else if flags_zero {
                        // zero-pad after the sign and 0x/0o prefix
                        let (sign, rest) = if let Some(r) = s.strip_prefix('-') {
                            ("-", r)
                        } else if let Some(r) = s.strip_prefix('+') {
                            ("+", r)
                        } else if let Some(r) = s.strip_prefix(' ') {
                            (" ", r)
                        } else {
                            ("", s.as_str())
                        };
                        let (prefix, body) = if let Some(r) = rest.strip_prefix("0x") {
                            ("0x", r)
                        } else if let Some(r) = rest.strip_prefix("0X") {
                            ("0X", r)
                        } else if let Some(r) = rest.strip_prefix("0o") {
                            ("0o", r)
                        } else {
                            ("", rest)
                        };
                        let padded = format!("{:0>width$}", body, width = w.saturating_sub(sign.len() + prefix.len()));
                        format!("{}{}{}", sign, prefix, padded)
                    } else {
                        format!("{:>width$}", s, width = w)
                    };
                }
                s.into_bytes()
            }
            b'e' | b'E' | b'f' | b'F' | b'g' | b'G' => {
                let raw = get_arg();
                // Must be a real number — a str arg raises TypeError
                // ("float argument required, not str"), not silently 0.0.
                let is_num = raw.as_f64().is_some() || matches!(&*raw.borrow(), PyObject::Int(_) | PyObject::Float(_));
                if !is_num {
                    return Err(format!(
                        "float argument required, not '{}'", raw.borrow().type_name()
                    ));
                }
                let f = raw.as_f64().unwrap_or(0.0);
                let p = precision.unwrap_or(6);
                if p > 200000 { return Err("precision too big".to_string()); }
                let s = match conv {
                    b'e' | b'E' => crate::object::format_percent_e(f, p, flags_alt, conv == b'E'),
                    b'g' | b'G' => {
                        let mut s = crate::object::format_percent_g(f, p, flags_alt);
                        if conv == b'G' { s = s.to_uppercase(); }
                        s
                    }
                    _ => {
                        let mut s = format_fixed_padded(f, p);
                        if flags_alt && !s.contains('.') { s.push('.'); }
                        s
                    }
                };
                let mut s = s.into_bytes();
                if let Some(w) = width {
                    let s_str = String::from_utf8_lossy(&s).to_string();
                    let padded = if flags_minus {
                        format!("{:<width$}", s_str, width = w)
                    } else if flags_zero {
                        format!("{:0>width$}", s_str, width = w)
                    } else {
                        format!("{:>width$}", s_str, width = w)
                    };
                    s = padded.into_bytes();
                }
                s
            }
            b'c' => {
                let raw = get_arg();
                let out = if let Some(n) = raw.as_i64() {
                    if n < 0 || n > 255 { return Err("%c arg not in range(256) [overflow]".to_string()); }
                    vec![n as u8]
                } else if matches!(&*raw.borrow(), PyObject::Int(_) | PyObject::Bool(_)) {
                    // A big int (e.g. 2**128) is out of range(256).
                    return Err("%c arg not in range(256) [overflow]".to_string());
                } else if let Some(b) = as_bytes_like(&raw) {
                    if b.len() != 1 { return Err("%c requires an integer in range(256) or a single byte, not a bytes object of length {}".to_string()); }
                    b
                } else if matches!(&*raw.borrow(), PyObject::Str(_)) {
                    return Err("%c requires an integer in range(256) or a single byte, not str".to_string());
                } else {
                    return Err("%c requires an integer in range(256) or a single byte".to_string());
                };
                if let Some(w) = width {
                    if out.len() < w {
                        let pad = w - out.len();
                        if flags_minus {
                            let mut v = out.clone();
                            v.extend(std::iter::repeat(b' ').take(pad));
                            v
                        } else {
                            let mut v = std::iter::repeat(b' ').take(pad).collect::<Vec<u8>>();
                            v.extend(out);
                            v
                        }
                    } else { out }
                } else { out }
            }
            c => return Err(format!("unsupported format character '{}' (0x{:02x}) at index {}",
                c as char, c as u32, i - 1)),
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

    // Real CPython raises if a single non-mapping arg is provided but no
    // conversion consumed it, or a tuple has MORE elements than specs.
    let arg_is_dict = matches!(&*arg0.borrow(), PyObject::Dict(_));
    if arg_iter.is_none() && converted == 0 && !arg_is_dict {
        return Err("not all arguments converted during bytes formatting".to_string());
    }
    if let Some(it) = arg_iter {
        let mut it = it;
        if it.next().is_some() {
            return Err("not all arguments converted during bytes formatting".to_string());
        }
    }
    Ok(result)
}
