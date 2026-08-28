// Extracted from pyobject.rs — PyObject::repr.
use super::*;

fn repr_string(s: &str) -> String {
    let has_single = s.contains('\'');
    let has_double = s.contains('"');
    let quote = if has_single && !has_double {
        '"'
    } else {
        '\''
    };
    let mut out = String::with_capacity(s.len() + 2);
    // Use same is_printable logic as escape_string
    fn is_printable(c: char) -> bool {
        if c.is_ascii() {
            return c >= ' ' && c != '\x7f';
        }
        if c.is_alphanumeric() || c.is_whitespace() {
            return true;
        }
        let cp = c as u32;
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
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            c if c == quote => {
                out.push('\\');
                out.push(quote);
            }
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            '\x00'..='\x1f' => out.push_str(&format!("\\x{:02x}", c as u8)),
            '\x7f' => out.push_str("\\x7f"),
            c if c.is_control() => match c as u32 {
                code @ 0..=0xff => out.push_str(&format!("\\x{:02x}", code as u8)),
                code @ 0x100..=0xffff => out.push_str(&format!("\\u{:04x}", code)),
                code => out.push_str(&format!("\\U{:08x}", code)),
            },
            c if !is_printable(c) => match c as u32 {
                code @ 0x100..=0xffff => out.push_str(&format!("\\u{:04x}", code)),
                code => out.push_str(&format!("\\U{:08x}", code)),
            },
            c => out.push(c),
        }
    }
    format!("{}{}{}", quote, out, quote)
}

impl PyObject {
    pub fn repr(&self) -> String {
        match self {
            PyObject::None => "None".to_string(),
            PyObject::Bool(b) => if *b { "True" } else { "False" }.to_string(),
            PyObject::Int(i) => i.to_string(),
            PyObject::Float(f) => format_py_float(*f),
            PyObject::Complex(re, im) => {
                // Real CPython: a zero real part reprs as just `<imag>j`;
                // otherwise `(<real><sign><|imag|>j)` — matches `repr(1+2j)`
                // == '(1+2j)', `repr(2j)` == '2j', `repr(1-2j)` == '(1-2j)'.
                if *re == 0.0 && re.is_sign_positive() {
                    format!("{}j", format_complex_part(*im))
                } else {
                    let sign = if im.is_sign_negative() { "-" } else { "+" };
                    format!(
                        "({}{}{}j)",
                        format_complex_part(*re),
                        sign,
                        format_complex_part(im.abs())
                    )
                }
            }
            PyObject::Str(s) => repr_string(s),
            PyObject::Bytes(b) => {
                let s: String = b
                    .iter()
                    .map(|&byte| match byte {
                        b'\\' => "\\\\".to_string(),
                        b'\'' => "\\'".to_string(),
                        b'\n' => "\\n".to_string(),
                        b'\t' => "\\t".to_string(),
                        b'\r' => "\\r".to_string(),
                        0x20..=0x7e => (byte as char).to_string(),
                        _ => format!("\\x{:02x}", byte),
                    })
                    .collect();
                format!("b'{}'", s)
            }
            PyObject::ByteArray(b) => {
                let s: String = b
                    .iter()
                    .map(|&byte| match byte {
                        b'\\' => "\\\\".to_string(),
                        b'\'' => "\\'".to_string(),
                        b'\n' => "\\n".to_string(),
                        b'\t' => "\\t".to_string(),
                        b'\r' => "\\r".to_string(),
                        0x20..=0x7e => (byte as char).to_string(),
                        _ => format!("\\x{:02x}", byte),
                    })
                    .collect();
                format!("bytearray(b'{}')", s)
            }
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
            PyObject::DequeIter { .. } => "<deque_iterator object>".to_string(),
            PyObject::DequeRevIter { .. } => "<deque_reverse_iterator object>".to_string(),
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
            PyObject::Module { name, .. } => format!("<module '{}'>", name),
            PyObject::Type { name, .. } => format!("<class '{}'>", name),
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
                                let items_str: Vec<String> =
                                    vec.iter().map(|x| x.repr()).collect();
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
                                let items_str: Vec<String> =
                                    vec.iter().map(|x| x.repr()).collect();
                                let inner = items_str.join(", ");
                                format!("{}({{{}}})", cls_name, inner)
                            }
                        }
                        PyObject::List(items) => {
                            let items_str: Vec<String> =
                                items.iter().map(|x| x.repr()).collect();
                            let inner = items_str.join(", ");
                            format!("[{}]", inner)
                        }
                        PyObject::Tuple(items) => {
                            let items_str: Vec<String> =
                                items.iter().map(|x| x.repr()).collect();
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
                            let inner = repr_string(s);
                            if cls_name == "str" {
                                inner
                            } else {
                                format!("{}({})", cls_name, inner)
                            }
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
                    let (tname, tptr) = {
                        let b = rc.borrow();
                        // Stable identity address of the target PyObject
                        (b.type_name(), std::ptr::from_ref::<PyObject>(&*b) as usize)
                    };
                    format!(
                        "<weakref at {:#x}; to '{}' at {:#x}>",
                        std::ptr::from_ref::<PyObject>(self) as usize,
                        tname,
                        tptr
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
                let args_str: Vec<String> = args.iter().map(|a| a.repr()).collect();
                let exc_str: Vec<String> = exceptions.iter().map(|e| e.repr()).collect();
                format!("{}({}, {})", typ, args_str.join(", "), exc_str.join(", "))
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
                            .unwrap_or_else(|| crate::interner::lookup_str(f.code.name).to_string());
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
                format!(
                    "<bound method {} of {}>",
                    mname,
                    self_obj.repr()
                )
            }
            PyObject::Partial { func, .. } => format!("<partial {}>", func.borrow().type_name()),
            PyObject::File { name, .. } => format!("<_io.FileIO '{}'>", name),
            PyObject::Socket { .. } => format!("<socket object>"),
            PyObject::Thread(_) => "<Thread>".to_string(),
            PyObject::Lock(_) => "<lock>".to_string(),
            PyObject::RLock(_) => "<RLock>".to_string(),
            PyObject::Event(_) => "<Event>".to_string(),
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
            PyObject::CompiledRegex { pattern, .. } => format!("re.compile('{}')", pattern),
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
