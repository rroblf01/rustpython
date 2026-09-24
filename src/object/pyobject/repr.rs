// Extracted from pyobject.rs — PyObject::repr.
use super::*;
use crate::object::pystring::PyStr;

fn repr_pstr(s: &PyStr) -> String {
    let has_single = s.as_str().contains('\'');
    let has_double = s.as_str().contains('"');
    let quote = if has_single && !has_double { '"' } else { '\'' };
    let mut out = String::with_capacity(s.len() + 2);
    // Use same is_printable logic as escape_string
    fn is_printable_cp(cp: u32) -> bool {
        if cp < 0x80 {
            return cp >= b' ' as u32 && cp != 0x7f;
        }
        if char::from_u32(cp).map_or(false, |c| c.is_alphanumeric())
            || cp == 0x09
            || cp == 0x0A
            || cp == 0x0D
            || (0x2000..=0x200E).contains(&cp)
        {
            return true;
        }
        matches!(cp,
            0x00A0..=0x00FF
            | 0x2000..=0x206F
            | 0x2100..=0x214F
            | 0x2190..=0x2BFF
            | 0x2E00..=0x2E7F
            | 0x3000..=0x303F
            | 0xFE50..=0xFE6F
            | 0xFF00..=0xFFEF
        )
    }
    fn is_control_cp(cp: u32) -> bool {
        matches!(cp, 0x00..=0x1F | 0x7F | 0x80..=0x9F)
    }
    // Iterate over WTF-8 codepoints (including surrogates).
    // For Utf8 variant, chars() returns u32 codepoints.
    // For Wtf8 variant, we decode the raw bytes.
    let codepoints: Vec<u32> = if s.has_surrogates() {
        crate::object::pystring::Wtf8Chars::new(s.as_bytes()).collect()
    } else {
        s.as_str().chars().map(|c| c as u32).collect()
    };
    for &cp in &codepoints {
        // Escape the quote char itself (check against both single/double).
        let cp_char = char::from_u32(cp);
        let is_quote = cp_char == Some(quote as char);
        match cp {
            0x5C => out.push_str("\\\\"),
            c if is_quote => {
                out.push('\\');
                out.push(quote);
            }
            0x0A => out.push_str("\\n"),
            0x09 => out.push_str("\\t"),
            0x0D => out.push_str("\\r"),
            0x00..=0x1F => out.push_str(&format!("\\x{:02x}", cp as u8)),
            0x7F => out.push_str("\\x7f"),
            c if is_control_cp(c) => match c {
                0x00..=0xFF => out.push_str(&format!("\\x{:02x}", c as u8)),
                0x100..=0xFFFF => out.push_str(&format!("\\u{:04x}", c)),
                _ => out.push_str(&format!("\\U{:08x}", c)),
            },
            // Lone surrogates (0xD800-0xDFFF) are always escaped.
            0xD800..=0xDFFF => out.push_str(&format!("\\u{:04x}", cp)),
            c if !is_printable_cp(c) => match c {
                0x100..=0xFFFF => out.push_str(&format!("\\u{:04x}", c)),
                _ => out.push_str(&format!("\\U{:08x}", c)),
            },
            c => {
                if let Some(ch) = char::from_u32(c) {
                    out.push(ch);
                } else {
                    // Shouldn't happen for valid WTF-8 codepoints.
                    out.push_str(&format!("\\u{:04x}", c));
                }
            }
        }
    }
    format!("{}{}{}", quote, out, quote)
}

fn repr_bytes_inner(b: &[u8]) -> String {
    let mut s = String::with_capacity(b.len());
    for &byte in b {
        match byte {
            b'\\' => s.push_str("\\\\"),
            b'\'' => s.push_str("\\'"),
            b'\n' => s.push_str("\\n"),
            b'\t' => s.push_str("\\t"),
            b'\r' => s.push_str("\\r"),
            0x20..=0x7e => s.push(byte as char),
            _ => s.push_str(&format!("\\x{:02x}", byte)),
        }
    }
    s
}

impl PyObject {
    pub fn repr(&self) -> String {
        match self {
            PyObject::None => "None".to_string(),
            PyObject::Bool(b) => if *b { "True" } else { "False" }.to_string(),
            PyObject::Int(i) => i.to_string(),
            // IntFlag repr (`re.IGNORECASE`, `re.A|re.B|0xNN`, ...).
            PyObject::RegexFlag(v) => super::super::regexflag_repr(*v),
            PyObject::Float(f) => format_py_float(*f),
            PyObject::Complex(re, im) => {
                // Real CPython: a zero real part reprs as just `<imag>j`;
                // otherwise `(<real><sign><|imag|>j)` — matches `repr(1+2j)`
                // == '(1+2j)', `repr(2j)` == '2j', `repr(1-2j)` == '(1-2j)'.
                if *re == 0.0 && re.is_sign_positive() {
                    format!("{}j", format_complex_part(*im))
                } else {
                    // Sign for the imag part: `-` for genuinely negative
                    // values (incl. -0.0), but `+` for ANY nan (CPython:
                    // `repr(complex(-nan,-nan))` == "(nan+nanj)", not
                    // "(nan-nanj)") — `is_sign_negative()` is True for -nan,
                    // so nan must be excluded.
                    let sign = if im.is_sign_negative() && !im.is_nan() {
                        "-"
                    } else {
                        "+"
                    };
                    format!(
                        "({}{}{}j)",
                        format_complex_part(*re),
                        sign,
                        format_complex_part(im.abs())
                    )
                }
            }
            PyObject::Str(s) => repr_pstr(s),
            PyObject::Bytes(b) => format!("b'{}'", repr_bytes_inner(b)),
            PyObject::ByteArray(b) => format!("bytearray(b'{}')", repr_bytes_inner(b)),
            PyObject::List(items) => {
                let items: Vec<String> = items.iter().map(|x| x.repr()).collect();
                format!("[{}]", items.join(", "))
            }
            PyObject::Deque { data, maxlen } => {
                let items: Vec<String> = data.iter().map(|x| x.repr()).collect();
                match maxlen {
                    Some(n) => format!("deque([{}], maxlen={})", items.join(", "), n),
                    None => format!("deque([{}])", items.join(", ")),
                }
            }
            PyObject::Tuple(items) => {
                let items: Vec<String> = items.iter().map(|x| x.repr()).collect();
                if items.len() == 1 {
                    format!("({},)", items[0])
                } else {
                    format!("({})", items.join(", "))
                }
            }
            PyObject::Dict(d) => {
                let items: Vec<String> = d
                    .items()
                    .iter()
                    .map(|(k, v)| format!("{}: {}", k.repr(), v.repr()))
                    .collect();
                format!("{{{}}}", items.join(", "))
            }
            PyObject::Globals(g) => {
                let entries: Vec<(PyObjectRef, PyObjectRef)> = g
                    .borrow()
                    .iter()
                    .map(|(k, v)| (py_str(interner::lookup_str(*k)), v.clone()))
                    .collect();
                let parts: Vec<String> = entries
                    .iter()
                    .map(|(k, v)| format!("{}: {}", k.repr(), v.repr()))
                    .collect();
                format!("{{{}}}", parts.join(", "))
            }
            PyObject::Set(items) => {
                let vec = items.to_vec();
                if vec.is_empty() {
                    "set()".to_string()
                } else {
                    let items: Vec<String> = vec.iter().map(|x| x.repr()).collect();
                    format!("{{{}}}", items.join(", "))
                }
            }
            PyObject::FrozenSet(items) => {
                let vec = items.to_vec();
                if vec.is_empty() {
                    "frozenset()".to_string()
                } else {
                    let items: Vec<String> = vec.iter().map(|x| x.repr()).collect();
                    format!("frozenset({{{}}})", items.join(", "))
                }
            }
            PyObject::Range { start, stop, step } => {
                if *step == num_bigint::BigInt::from(1) {
                    format!("range({}, {})", start, stop)
                } else {
                    format!("range({}, {}, {})", start, stop, step)
                }
            }
            PyObject::RangeIter { .. } => "<range_iterator object>".to_string(),
            PyObject::ListIter { .. } => "<list_iterator object>".to_string(),
            PyObject::FindIter { .. } => "<callable_iterator object>".to_string(),
            PyObject::DequeIter { .. } => "<deque_iterator object>".to_string(),
            PyObject::DequeRevIter { .. } => "<deque_reverse_iterator object>".to_string(),
            PyObject::DictIter { .. } => "<dict_keyiterator object>".to_string(),
            PyObject::DictValuesIter { .. } => "<dict_valueiterator object>".to_string(),
            PyObject::DictItemsIter { .. } => "<dict_itemiterator object>".to_string(),
            PyObject::DictRevIter { .. } => "<dict_reversekeyiterator object>".to_string(),
            PyObject::GetItemIter { .. } => "<iterator object>".to_string(),
            PyObject::CallSentinelIter { .. } => "<callable_iterator object>".to_string(),
            PyObject::EnumerateIter { .. } => "<enumerate object>".to_string(),
            PyObject::MapIterator { .. } => "<map object>".to_string(),
            PyObject::FilterIterator { .. } => "<filter object>".to_string(),
            PyObject::ZipIterator { .. } => "<zip object>".to_string(),
            PyObject::Slice { start, stop, step } => {
                format!("slice({}, {}, {})", start.repr(), stop.repr(), step.repr())
            }
            PyObject::Function(ref f) => format!("<function {}>", f.code.name),
            // A builtin exception "class" (`ValueError`, `OSError`, ...) is
            // represented as a `BuiltinFunction` marker in this codebase
            // (see `is_builtin_exception_class_name`'s own doc comments),
            // but it IS a real class from Python's perspective — its repr
            // must read like one (`<class 'ValueError'>`), not like an
            // ordinary function (`<built-in function ValueError>`). Real
            // trigger: `cmd`'s doctest printing `sys.exc_info()[0]`
            // (test_cmd.py's CmdPrintExceptionClass).
            PyObject::BuiltinFunction { name, .. }
                if crate::object::is_builtin_exception_class_name(name) =>
            {
                format!("<class '{}'>", name)
            }
            // Same representation gap as builtin exceptions above, for a
            // different family: `property`/`staticmethod`/`classmethod`/
            // `super` are real classes from Python's perspective (each is
            // literally usable as a base class) but are likewise
            // represented as a `BuiltinFunction` marker in this codebase,
            // not a real `PyObject::Type` — deliberately NOT folded into
            // `is_builtin_exception_class_name` itself, since that name is
            // also used for exception-ancestry checks elsewhere and these
            // are not exceptions (real trigger: test_descrtut.py's `del
            // property # unmask the builtin` doctest, which expects
            // `<class 'property'>` once the builtin is revealed).
            PyObject::BuiltinFunction { name, .. }
                if matches!(
                    name.as_str(),
                    "property" | "staticmethod" | "classmethod" | "super"
                ) =>
            {
                format!("<class '{}'>", name)
            }
            PyObject::BuiltinFunction { name, .. } => format!("<built-in function {}>", name),
            PyObject::BuiltinMethod { name, self_obj, .. } => {
                // CPython: `<built-in method split of str object at 0x...>`
                // — a method bound to a native receiver reports the
                // receiver's type (test_reprlib::test_builtin_function).
                let receiver = self_obj.borrow();
                let owner = if matches!(&*receiver, PyObject::None) {
                    None
                } else {
                    Some(receiver.type_name().to_string())
                };
                match owner {
                    Some(t) => format!(
                        "<built-in method {} of {} object at 0x{:x}>",
                        name, t, self as *const PyObject as usize
                    ),
                    None => format!("<built-in method {}>", name),
                }
            }
            PyObject::Module { name, dict } => match dict.get_str("__file__") {
                // Mirrors CPython: `<module 'unittest' from '.../__init__.py'>`.
                Some(f) => format!("<module '{}' from '{}'>", name, f),
                None => format!("<module '{}'>", name),
            },
            PyObject::Type { name, dict, .. } => {
                // Real CPython: `<class 'module.Name'>` unless the module is
                // `builtins` (or unset), which prints the bare name instead.
                let module = dict.get_str("__module__").map(|m| m.str());
                match module.as_deref() {
                    Some("builtins") | None => format!("<class '{}'>", name),
                    Some(m) => format!("<class '{}.{}'>", m, name),
                }
            }
            PyObject::Instance { typ, dict } => {
                // For native-base subclasses (e.g. class Foo(set): ...), the instance
                // has a native backing (PyObject::Set/List/Dict etc) stored under
                // NATIVE_BACKING_KEY. Its repr should delegate to the backing with
                // the subclass name, not the generic "<Foo object at ...>".
                if let Some(native) = dict.get(NATIVE_BACKING_KEY) {
                    let native_borrow = native.borrow();
                    let tb = typ.borrow();
                    let cls_name = if let PyObject::Type { name, .. } = &*tb {
                        name.clone()
                    } else {
                        tb.type_name().to_string()
                    };
                    match &*native_borrow {
                        PyObject::Set(items) => {
                            let vec = items.to_vec();
                            if vec.is_empty() {
                                if cls_name == "set" {
                                    "set()".to_string()
                                } else {
                                    format!("{}()", cls_name)
                                }
                            } else {
                                let items_str: Vec<String> = vec.iter().map(|x| x.repr()).collect();
                                let inner = items_str.join(", ");
                                if cls_name == "set" {
                                    format!("{{{}}}", inner)
                                } else {
                                    format!("{}({{{}}})", cls_name, inner)
                                }
                            }
                        }
                        PyObject::FrozenSet(items) => {
                            let vec = items.to_vec();
                            if vec.is_empty() {
                                if cls_name == "frozenset" {
                                    "frozenset()".to_string()
                                } else {
                                    format!("{}()", cls_name)
                                }
                            } else {
                                let items_str: Vec<String> = vec.iter().map(|x| x.repr()).collect();
                                let inner = items_str.join(", ");
                                format!("{}({{{}}})", cls_name, inner)
                            }
                        }
                        PyObject::List(items) => {
                            let items_str: Vec<String> = items.iter().map(|x| x.repr()).collect();
                            let inner = items_str.join(", ");
                            format!("[{}]", inner)
                        }
                        PyObject::Tuple(items) => {
                            let items_str: Vec<String> = items.iter().map(|x| x.repr()).collect();
                            let inner = items_str.join(", ");
                            if items_str.len() == 1 {
                                format!("({},)", items_str[0])
                            } else {
                                format!("({})", inner)
                            }
                        }
                        PyObject::Dict(d) => {
                            let items: Vec<String> = d
                                .items()
                                .iter()
                                .map(|(k, v)| format!("{}: {}", k.repr(), v.repr()))
                                .collect();
                            let inner = items.join(", ");
                            format!("{{{}}}", inner)
                        }
                        PyObject::Str(s) => {
                            let inner = repr_pstr(s);
                            inner
                        }
                        _ => {
                            let name = if let PyObject::Type { dict, name, .. } = &*tb {
                                let module = dict
                                    .get_str("__module__")
                                    .map(|m| m.str())
                                    .unwrap_or_else(|| "builtins".to_string());
                                format!("{}.{}", module, name)
                            } else {
                                tb.type_name().to_string()
                            };
                            format!(
                                "<{} object at 0x{:x}>",
                                name, self as *const PyObject as usize
                            )
                        }
                    }
                } else {
                    // CPython: `<module.Class object at 0x...>` — dataclasses'
                    // repr=False instances and test_pprint's regex expect the
                    // module-qualified name, not the bare `<Class object>`.
                    let tb = typ.borrow();
                    let name = if let PyObject::Type { dict, name, .. } = &*tb {
                        let module = dict
                            .get_str("__module__")
                            .map(|m| m.str())
                            .unwrap_or_else(|| "builtins".to_string());
                        format!("{}.{}", module, name)
                    } else {
                        tb.type_name().to_string()
                    };
                    format!(
                        "<{} object at 0x{:x}>",
                        name, self as *const PyObject as usize
                    )
                }
            }
            PyObject::Code(c) => format!("<code object {}>", c.name),
            PyObject::Cell { value: Some(v) } => v.repr(),
            PyObject::Cell { value: None } => "None".to_string(),
            PyObject::WeakRef { target, .. } => match target.upgrade() {
                Some(rc) => {
                    let (tname, tptr, suffix) = {
                        let rc_ref = PyObjectRef::Imm(rc.clone());
                        let name = {
                            if let Ok(cls) = rc_ref.borrow().get_attribute("__class__") {
                                if let Ok(n) = cls.borrow().get_attribute("__name__") {
                                    if let PyObject::Str(s) = &*n.borrow() {
                                        s.to_string()
                                    } else {
                                        rc_ref.borrow().type_name().to_string()
                                    }
                                } else {
                                    rc_ref.borrow().type_name().to_string()
                                }
                            } else {
                                rc_ref.borrow().type_name().to_string()
                            }
                        };
                        let ptr = std::ptr::from_ref::<PyObject>(&*rc.borrow()) as usize;
                        let suffix = {
                            if let Ok(n) = rc_ref.borrow().get_attribute("__name__") {
                                if let PyObject::Str(s) = &*n.borrow() {
                                    format!(" ({})", s)
                                } else {
                                    String::new()
                                }
                            } else {
                                String::new()
                            }
                        };
                        (name, ptr, suffix)
                    };
                    format!(
                        "<weakref at {:#x}; to '{}' at {:#x}{}>",
                        std::ptr::from_ref::<PyObject>(self) as usize,
                        tname,
                        tptr,
                        suffix
                    )
                }
                None => format!(
                    "<weakref at {:#x}; dead>",
                    std::ptr::from_ref::<PyObject>(self) as usize
                ),
            },
            PyObject::WeakProxy { target, .. } => match target.upgrade() {
                Some(rc) => rc.borrow().repr(),
                None => format!(
                    "<weakproxy at {:#x}; dead>",
                    std::ptr::from_ref::<PyObject>(self) as usize
                ),
            },
            PyObject::Capsule { name, .. } => format!("<capsule object '{}'>", name),
            PyObject::Exception {
                typ,
                args,
                cause: _,
                suppress_context: _,
                ..
            } => {
                let args_str: Vec<String> = args.iter().map(|a| a.repr()).collect();
                format!("{}({})", typ, args_str.join(", "))
            }
            PyObject::ExceptionGroup {
                typ,
                args,
                exceptions,
            } => {
                // CPython's constructor-shaped repr:
                // `ExceptionGroup('m', [ValueError('a'), TypeError('b')])` —
                // the message plus the children as a LIST. `args` already
                // holds `(message, [children])` (both the ctor and
                // `derive`/`split` maintain that shape), so print it
                // directly; the previous version appended `exceptions` a
                // SECOND time, duplicating every child.
                let args_str: Vec<String> = args.iter().map(|a| a.repr()).collect();
                if args.len() >= 2 {
                    format!("{}({})", typ, args_str.join(", "))
                } else {
                    // Defensive: a hand-built group with no children list in
                    // `args` still reprs its children rather than nothing.
                    let exc_str: Vec<String> = exceptions.iter().map(|e| e.repr()).collect();
                    format!("{}({}, [{}])", typ, args_str.join(", "), exc_str.join(", "))
                }
            }
            PyObject::BuildClass => "<builtin function __build_class__>".to_string(),
            PyObject::BoundMethod { func, self_obj } => {
                // CPython-style: <bound method Class.method of <owner repr>>.
                // Method name prefers the function's __qualname__; when it
                // carries no class prefix, synthesize one from the owner's
                // type name.
                let fb = func.borrow();
                let mname = match &*fb {
                    PyObject::Function(f) => {
                        let qn = f
                            .dict
                            .get("__qualname__")
                            .and_then(|v| {
                                let b = v.borrow();
                                if let PyObject::Str(s) = &*b {
                                    Some(s.to_string())
                                } else {
                                    None
                                }
                            })
                            .unwrap_or_else(|| {
                                crate::interner::lookup_str(f.code.name).to_string()
                            });
                        // Prefer the user-visible CLASS name: instances of
                        // Python-level classes report generic 'instance' as
                        // their runtime type_name, but CPython's bound-method
                        // repr uses the class qualifier (sub._factory).
                        let tn = match &*self_obj.borrow() {
                            PyObject::Instance { typ, .. } => {
                                let tb = typ.borrow();
                                match &*tb {
                                    PyObject::Type { name, .. } => name.clone(),
                                    _ => tb.type_name().to_string(),
                                }
                            }
                            other => other.type_name().to_string(),
                        };
                        let tn = tn.as_str();
                        if qn.contains('.') || qn == tn {
                            qn
                        } else {
                            format!("{}.{}", tn, qn)
                        }
                    }
                    _ => fb.type_name(),
                };
                drop(fb);
                format!("<bound method {} of {}>", mname, self_obj.repr())
            }
            PyObject::Partial { func, .. } => format!("<partial {}>", func.borrow().type_name()),
            PyObject::File { name, .. } => format!("<_io.FileIO '{}'>", name),
            PyObject::Socket { .. } => format!("<socket object>"),
            PyObject::Thread(_) => "<Thread>".to_string(),
            // Real CPython: `<locked _thread.lock object at 0x...>` /
            // `<unlocked _thread.lock object at 0x...>` — was a bare
            // `"<lock>"` with no locked-state or address at all, failing
            // `Lib/test/lock_tests.py`'s `test_locked_repr`
            // (`assertRegex(repr(lock), "<locked .* object (.*)?at .*>")`,
            // used by both `test_thread.py` and `test_threading.py`).
            PyObject::Lock(inner_arc) => {
                let locked = inner_arc
                    .lock()
                    .map(|g| g.lock.load(std::sync::atomic::Ordering::SeqCst))
                    .unwrap_or(false);
                format!(
                    "<{} _thread.lock object at 0x{:x}>",
                    if locked { "locked" } else { "unlocked" },
                    self as *const PyObject as usize
                )
            }
            PyObject::RLock(inner_arc) => {
                let locked = inner_arc.lock().map(|g| g.owner.is_some()).unwrap_or(false);
                format!(
                    "<{} _thread.RLock object owner=0 count=0 at 0x{:x}>",
                    if locked { "locked" } else { "unlocked" },
                    self as *const PyObject as usize
                )
            }
            PyObject::Event(_) => "<Event>".to_string(),
            PyObject::ThreadCondition { .. } => "<Condition>".to_string(),
            PyObject::ThreadSemaphore { .. } => "<Semaphore>".to_string(),
            PyObject::ThreadBarrier { .. } => "<Barrier>".to_string(),
            PyObject::ThreadTimer { .. } => "<Timer>".to_string(),
            PyObject::Queue(_) => "<Queue>".to_string(),
            PyObject::Super { .. } => format!("<super object>"),
            PyObject::Property(_) => format!("<property object>"),
            PyObject::StaticMethod { func } => format!("<staticmethod({})>", func.repr()),
            PyObject::ClassMethod { func } => format!("<classmethod({})>", func.repr()),
            PyObject::Generator { .. } => format!("<generator object>"),
            PyObject::Coroutine { .. } => format!("<coroutine object>"),
            PyObject::Array(arr) => {
                let items: Vec<String> = arr
                    .data
                    .iter()
                    .map(|v| {
                        if array_typecode_is_float(arr.typecode) {
                            py_float(*v).repr()
                        } else {
                            py_int(*v as i64).repr()
                        }
                    })
                    .collect();
                if items.is_empty() {
                    // CPython: an empty array reprs as `array('i')`.
                    format!("array('{}')", arr.typecode)
                } else {
                    format!("array('{}', [{}])", arr.typecode, items.join(", "))
                }
            }
            PyObject::MemoryView { .. } => {
                format!("<memory at 0x{:012x}>", self as *const PyObject as usize)
            }
            PyObject::CompiledRegex {
                pattern,
                flags,
                is_bytes,
                ..
            } => {
                let pattern_repr = if *is_bytes {
                    format!("b'{}'", repr_bytes_inner(pattern.as_bytes()))
                } else {
                    repr_pstr(&PyStr::from_str(pattern))
                };
                // CPython's `%.200R` — truncate the pattern repr at 200
                // chars, appending `...`.
                let pattern_repr: String = if pattern_repr.chars().count() > 200 {
                    pattern_repr
                        .chars()
                        .take(200)
                        .chain("...".chars())
                        .collect()
                } else {
                    pattern_repr
                };
                // Omit re.UNICODE for valid string patterns (same
                // condition as CPython's `pattern_repr`).
                let mut flags = *flags;
                if !*is_bytes && (flags & (4 | 32 | 256)) == 32 {
                    flags &= !32;
                }
                let mut parts: Vec<String> = Vec::new();
                macro_rules! push_flag {
                    ($bit:expr, $name:literal) => {
                        if flags & $bit != 0 {
                            parts.push($name.to_string());
                            flags &= !$bit;
                        }
                    };
                }
                push_flag!(2, "re.IGNORECASE");
                push_flag!(4, "re.LOCALE");
                push_flag!(8, "re.MULTILINE");
                push_flag!(16, "re.DOTALL");
                push_flag!(32, "re.UNICODE");
                push_flag!(64, "re.VERBOSE");
                push_flag!(128, "re.DEBUG");
                push_flag!(256, "re.ASCII");
                if flags != 0 {
                    parts.push(format!("0x{:x}", flags));
                }
                if parts.is_empty() {
                    format!("re.compile({})", pattern_repr)
                } else {
                    format!("re.compile({}, {})", pattern_repr, parts.join("|"))
                }
            }
            PyObject::Closure(_) => "<builtin function>".to_string(),
            PyObject::FutureAwaitIterator { future, yielded } => {
                format!(
                    "<future_await_iterator future={} yielded={}>",
                    future.repr(),
                    yielded
                )
            }
            PyObject::Process {
                pid, returncode, ..
            } => {
                format!(
                    "<Popen: returncode: {} args: [pid {}]>",
                    returncode
                        .borrow()
                        .map(|r| r.to_string())
                        .unwrap_or_else(|| "None".to_string()),
                    pid
                )
            }
            PyObject::CycleIter { .. } => "<itertools.cycle object>".to_string(),
            PyObject::GroupByIter { .. } => "<itertools.groupby object>".to_string(),
        }
    }
}
