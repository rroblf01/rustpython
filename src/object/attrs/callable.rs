// Extracted from src/object/attrs/mod.rs — Function/BoundMethod/BuiltinFunction/Code
use crate::object::*;
use super::*;
use crate::interner;
use std::cell::RefCell;
use num_traits::ToPrimitive;

fn code_replace_get_kwarg(kwargs: &Option<PyObjectRef>, name: &str) -> Option<PyObjectRef> {
    kwargs.as_ref().and_then(|d| {
        if let PyObject::Dict(d) = &*d.borrow() {
            d.get(&py_str(name)).ok().flatten()
        } else {
            None
        }
    })
}

pub(crate) fn code_replace(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let self_obj = args.first().ok_or_else(|| {
        PyError::type_error("code.replace() missing self")
    })?;
    let kwargs: Option<PyObjectRef> = match args.get(1) {
        Some(last) if matches!(&*last.borrow(), PyObject::Dict(_)) => Some(last.clone()),
        _ => None,
    };
    let orig = match &*self_obj.borrow() {
        PyObject::Code(c) => (**c).clone(),
        _ => return Err(PyError::type_error("replace() self must be a code object")),
    };
    let mut new_code = orig;

    macro_rules! str_field {
        ($key:expr, $field:ident) => {
            if let Some(v) = code_replace_get_kwarg(&kwargs, $key) {
                new_code.$field = crate::interner::intern(&v.str());
            }
        };
    }
    macro_rules! usize_field {
        ($key:expr, $field:ident) => {
            if let Some(v) = code_replace_get_kwarg(&kwargs, $key) {
                if let PyObject::Int(n) = &*v.borrow() {
                    new_code.$field = n.to_usize().unwrap_or(0);
                }
            }
        };
    }

    str_field!("co_filename", filename);
    str_field!("co_name", name);
    usize_field!("co_argcount", arg_count);
    usize_field!("co_posonlyargcount", posonlyarg_count);
    usize_field!("co_kwonlyargcount", kwonlyarg_count);
    usize_field!("co_nlocals", nlocals);
    usize_field!("co_firstlineno", first_lineno);
    if let Some(v) = code_replace_get_kwarg(&kwargs, "co_flags") {
        if let PyObject::Int(n) = &*v.borrow() {
            new_code.flags = n.to_usize().unwrap_or(0) as u16;
        }
    }
    if let Some(v) = code_replace_get_kwarg(&kwargs, "co_varnames") {
        if let PyObject::Tuple(items) = &*v.borrow() {
            new_code.varnames = items
                .iter()
                .map(|s| crate::interner::intern(&s.str()))
                .collect();
        }
    }
    if let Some(v) = code_replace_get_kwarg(&kwargs, "co_names") {
        if let PyObject::Tuple(items) = &*v.borrow() {
            new_code.names = items
                .iter()
                .map(|s| crate::interner::intern(&s.str()))
                .collect();
        }
    }
    if let Some(v) = code_replace_get_kwarg(&kwargs, "co_consts") {
        if let PyObject::Tuple(items) = &*v.borrow() {
            new_code.consts = items
                .iter()
                .map(|item| crate::bytecode::ConstValue::Object(item.clone()))
                .collect();
        }
    }
    // Any lazily-populated per-const parse cache from the original code
    // object is invalid once `consts` may have changed — reset it so
    // `LOAD_CONST` re-derives fresh entries against the NEW `consts` vec
    // instead of serving stale cached values at the old indices.
    new_code.const_cache = RefCell::new(vec![None; new_code.consts.len()]);

    Ok(PyObjectRef::imm(PyObject::Code(std::rc::Rc::new(new_code))))
}

pub(crate) fn get(o: &PyObject, name: &str) -> PyResult<PyObjectRef> {
    match o {
            PyObject::Function(ref f) => {
                let func_name = &f.code.name;
                let dict = &f.dict;
                match name {
                    "__name__" => Ok(dict
                        .get("__name__")
                        .cloned()
                        .unwrap_or(py_str(crate::interner::lookup_str(*func_name)))),
                    "__qualname__" => Ok(dict
                        .get("__qualname__")
                        .cloned()
                        .unwrap_or_else(|| py_str(crate::interner::lookup_str(f.code.qualname)))),
                    "name" => Ok(dict
                        .get("name")
                        .cloned()
                        .unwrap_or(py_str(crate::interner::lookup_str(*func_name)))),
                    "__doc__" => Ok(dict.get("__doc__").cloned().unwrap_or(py_none())),
                    "__code__" => Ok(dict.get("__code__").cloned().unwrap_or(py_none())),
                    "__globals__" => Ok(dict.get("__globals__").cloned().unwrap_or(py_none())),
                    // Real `__defaults__`/`__kwdefaults__` introspection —
                    // was ALWAYS `None` regardless of the function's real
                    // signature (only reflected a value if user code
                    // explicitly assigned `f.__defaults__ = ...` by hand),
                    // even though the real default VALUES are already
                    // sitting right here on `f.defaults` (populated by
                    // `MAKE_FUNCTION`, which appends kwonly defaults after
                    // positional ones — see its own doc comment, `vm.rs`).
                    // `__kwdefaults__` additionally needs the kwonly
                    // parameter NAMES, which live in `varnames` right after
                    // the positional ones (`varnames[arg_count..][..
                    // kwonlyarg_count]` — standard CPython varnames layout).
                    // Missing entirely broke `test_keywordonlyarg.py::
                    // testKwDefaults` (`AttributeError` instead of a real
                    // dict).
                    "__defaults__" => {
                        if let Some(v) = dict.get("__defaults__") {
                            return Ok(v.clone());
                        }
                        let kwonly_with_default =
                            f.code.kwonly_defaults_mask.iter().filter(|&&b| b).count();
                        let pos_count = f.defaults.len().saturating_sub(kwonly_with_default);
                        if pos_count == 0 {
                            Ok(py_none())
                        } else {
                            Ok(py_tuple(f.defaults[..pos_count].to_vec()))
                        }
                    }
                    "__kwdefaults__" => {
                        if let Some(v) = dict.get("__kwdefaults__") {
                            return Ok(v.clone());
                        }
                        let kwonly_with_default =
                            f.code.kwonly_defaults_mask.iter().filter(|&&b| b).count();
                        if kwonly_with_default == 0 {
                            return Ok(py_none());
                        }
                        let pos_count = f.defaults.len().saturating_sub(kwonly_with_default);
                        let mut kw_d = PyDict::new();
                        let mut value_idx = pos_count;
                        for (i, &has_default) in f.code.kwonly_defaults_mask.iter().enumerate() {
                            if has_default {
                                if let Some(&name_id) = f.code.varnames.get(f.code.arg_count + i) {
                                    let arg_name = crate::interner::lookup_str(name_id);
                                    if let Some(val) = f.defaults.get(value_idx) {
                                        let _ = kw_d.set(py_str(arg_name), val.clone());
                                    }
                                }
                                value_idx += 1;
                            }
                        }
                        Ok(PyObjectRef::new(PyObject::Dict(Box::new(kw_d))))
                    }
                    "__closure__" => Ok(dict.get("__closure__").cloned().unwrap_or(py_none())),
                    "__module__" => Ok(dict.get("__module__").cloned().unwrap_or(py_none())),
                    "__annotations__" => {
                        // A directly-assigned value (`func.__annotations__ =
                        // {...}`) must win over whatever `__annotate__` would
                        // lazily compute — real CPython's `__annotations__`
                        // is a plain settable attribute, and code like
                        // `functools.update_wrapper` does exactly this
                        // (`setattr(wrapper, '__annotations__', value)`).
                        // Previously this fell straight through to the
                        // `__annotate__`/shared-empty-dict logic below and
                        // silently ignored any explicit assignment.
                        if let Some(v) = dict.get_str("__annotations__").cloned() {
                            return Ok(v);
                        }
                        // PEP 649: calling `__annotate__` lazily computes the
                        // annotations dict (undefined names fail only on
                        // first access). Cache per function (keyed by the
                        // __annotate__ closure's identity, or the function's
                        // own object address for the no-annotation empty
                        // dict) so repeated access returns the SAME dict —
                        // test_decorators asserts `func.__annotations__ is
                        // func.__annotations__`.
                        if let Some(annotate) = dict.get_str("__annotate__").cloned() {
                            // The decorator may have explicitly set
                            // `wrapper.__annotate__ = None` (reprlib's
                            // recursive_repr does) — None means "no lazy
                            // annotations", not a callable to invoke.
                            if !matches!(&*annotate.borrow(), PyObject::None) {
                                let key = annotate.get_id();
                                if let Some(cached) =
                                    ANN_CACHE.with(|c| c.borrow().get(&key).cloned())
                                {
                                    return Ok(cached);
                                }
                                let result = crate::object::call_function_disposable(
                                    &annotate,
                                    vec![],
                                    vec![],
                                )?;
                                ANN_CACHE.with(|c| c.borrow_mut().insert(key, result.clone()));
                                return Ok(result);
                            }
                        }
                        // No `__annotate__`: every annotation-less function
                        // shares ONE empty dict, so
                        // `func1.__annotations__ is func2.__annotations__`
                        // (test_reprlib::test_assigned_attributes asserts
                        // this across a wrapped function pair).
                        thread_local! {
                            static EMPTY_ANN: std::cell::RefCell<Option<PyObjectRef>> =
                                const { std::cell::RefCell::new(None) };
                        }
                        let empty = EMPTY_ANN.with(|c| {
                            let mut opt = c.borrow_mut();
                            if opt.is_none() {
                                *opt = Some(crate::object::py_dict());
                            }
                            opt.clone().unwrap()
                        });
                        Ok(empty)
                    }
                    // `func.__dict__` — every custom attribute set on a
                    // function (`f.custom = 1`) already lands in this same
                    // `dict` (see `set_attribute`'s `PyObject::Function`
                    // arm), but reading `__dict__` itself back out as a
                    // real dict was missing (`AttributeError`). Real
                    // trigger: CPython's own `test_funcattrs.py`-style
                    // checks of `f.__dict__`. Excludes the dunder slots
                    // above (`__name__`/`__doc__`/etc.) since real Python's
                    // `func.__dict__` only ever holds USER-set attributes,
                    // not those dedicated descriptor slots.
                    "__dict__" => {
                        let mut pd = PyDict::new();
                        for (k, v) in dict.iter() {
                            if k.starts_with("__") && k.ends_with("__") {
                                continue;
                            }
                            pd.set(py_str(k), v.clone())?;
                        }
                        Ok(PyObjectRef::new(PyObject::Dict(Box::new(pd))))
                    }
                    _ => dict.get_str(&name).cloned().ok_or_else(|| {
                        PyError::attribute_error(format!(
                            "'function' object has no attribute '{}'",
                            name
                        ))
                    }),
                }
            }
            PyObject::BoundMethod { func, self_obj } => {
                match name {
                    "__func__" => Ok(func.clone()),
                    "__self__" => Ok(self_obj.clone()),
                    // A real Python bound method proxies any attribute not
                    // found on the method object itself through to the
                    // underlying function (`__func__`) — this is how e.g.
                    // `SomeClass.some_classmethod.cache_clear()` reaches the
                    // functools.cache wrapper underneath the classmethod
                    // descriptor. Without this fallback, BoundMethod had no
                    // get_attribute arm at all and every such access raised
                    // "'method' object has no attribute ...".
                    //
                    // `func.get_attribute` alone (the ObjectAccess impl) does
                    // raw, unbound retrieval — it doesn't replicate LOAD_ATTR's
                    // self-binding for the result. Redo that binding here so
                    // e.g. `.cache_clear` comes back as a real bound call
                    // (self = func, the underlying cache-wrapper instance),
                    // not a plain unbound Function that would immediately hit
                    // "local variable 'self' referenced before assignment".
                    _ => {
                        let raw = func.borrow().get_attribute(name).map_err(|_| {
                            if std::env::var("RPY_DEBUG_ATTR").is_ok() {
                                let (fn_name, fn_file) = if let PyObject::Function(ref inner_f) = &*func.borrow() {
                                    let code = &inner_f.code;
                                    (code.name.to_string(), code.filename.to_string())
                                } else { ("?".to_string(), "?".to_string()) };
                                let self_kind = match &*self_obj.borrow() {
                                    PyObject::Type { name, .. } => format!("Type({})", name),
                                    other => format!("{}", other.type_name()),
                                };
                                eprintln!("BOUNDMETHOD_ATTR_FAIL: name={} func_kind={:?} fn_name={} fn_file={} self_kind={}", name, func.borrow().type_name(), fn_name, fn_file, self_kind);
                            }
                            PyError::attribute_error(format!(
                            "'method' object has no attribute '{}'", name
                        ))})?;
                        let is_instance_self = matches!(&*func.borrow(), PyObject::Instance { .. });
                        let raw_kind = {
                            let b = raw.borrow();
                            match &*b {
                                PyObject::Function { .. } if is_instance_self => 1,
                                PyObject::BuiltinFunction { .. } => 2,
                                PyObject::BuiltinMethod { .. } => 3,
                                _ => 0,
                            }
                        };
                        match raw_kind {
                            1 => Ok(PyObjectRef::imm(PyObject::BoundMethod {
                                func: raw,
                                self_obj: func.clone(),
                            })),
                            2 => {
                                let (n, f) = if let PyObject::BuiltinFunction { name: n, func: f } =
                                    &*raw.borrow()
                                {
                                    (n.clone(), *f)
                                } else {
                                    unreachable!()
                                };
                                Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                                    name: n,
                                    func: f,
                                    self_obj: func.clone(),
                                }))
                            }
                            3 => {
                                let (n, f) =
                                    if let PyObject::BuiltinMethod {
                                        name: n, func: f, ..
                                    } = &*raw.borrow()
                                    {
                                        (n.clone(), *f)
                                    } else {
                                        unreachable!()
                                    };
                                Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                                    name: n,
                                    func: f,
                                    self_obj: func.clone(),
                                }))
                            }
                            _ => Ok(raw),
                        }
                    }
                }
            }
            PyObject::BuiltinFunction {
                name: bf_name,
                func,
            } => {
                if bf_name == "memoryview" {
                    if name == "_from_flags" {
                        return Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                            name: "_from_flags".to_string(),
                            func: crate::object::mv_from_flags,
                            self_obj: PyObjectRef::new(PyObject::None),
                        }));
                    }
                }
                if bf_name == "bytes" && name == "fromhex" {
                    return Ok(PyObjectRef::imm(PyObject::BuiltinFunction {
                        name: "fromhex".to_string(),
                        func: builtin_bytes_fromhex,
                    }));
                }
                if bf_name == "complex" && name == "from_number" {
                    return Ok(PyObjectRef::imm(PyObject::BuiltinFunction {
                        name: "from_number".to_string(),
                        func: |args| {
                            if args.is_empty() {
                                return Err(PyError::type_error(
                                    "complex.from_number() takes exactly 1 argument",
                                ));
                            }
                            let n = args[0].as_f64().unwrap_or(0.0);
                            Ok(PyObjectRef::imm(PyObject::Complex(n, 0.0)))
                        },
                    }));
                }
                if bf_name == "float" && name == "__getformat__" {
                    // `float.__getformat__("double"/"float")` — real CPython
                    // queries the platform's actual float representation;
                    // this interpreter's floats are always IEEE 754 doubles
                    // (Rust `f64`), so always answer accordingly. Real
                    // trigger: CPython's own `test.support.requires_IEEE_754`
                    // module-level constant, `float.__getformat__("double").
                    // startswith("IEEE")`.
                    return Ok(PyObjectRef::imm(PyObject::BuiltinFunction {
                        name: "__getformat__".to_string(),
                        func: |_args| Ok(py_str("IEEE, little-endian")),
                    }));
                }
                if bf_name == "float" && name == "fromhex" {
                    return Ok(PyObjectRef::imm(PyObject::BuiltinFunction {
                        name: "fromhex".to_string(),
                        func: |args| {
                            if args.is_empty() {
                                return Err(PyError::type_error(
                                    "float.fromhex() requires exactly 1 argument",
                                ));
                            }
                            let s = args[0].str();
                            let s = s.trim();
                            let lower = s.to_lowercase();
                            if lower == "nan" {
                                return Ok(py_float(f64::NAN));
                            }
                            if lower == "inf"
                                || lower == "+inf"
                                || lower == "-inf"
                                || lower == "infinity"
                                || lower == "+infinity"
                                || lower == "-infinity"
                            {
                                let sign = if lower.starts_with('-') { -1.0 } else { 1.0 };
                                return Ok(py_float(sign * f64::INFINITY));
                            }
                            let s = s.strip_prefix("+").unwrap_or(s);
                            let sign = if s.starts_with('-') { -1.0 } else { 1.0 };
                            let s = s
                                .strip_prefix('-')
                                .unwrap_or(s.strip_prefix('+').unwrap_or(s));
                            let s = s
                                .strip_prefix("0x")
                                .or_else(|| s.strip_prefix("0X"))
                                .ok_or_else(|| {
                                    PyError::value_error(format!(
                                        "invalid hex float literal: {}",
                                        s
                                    ))
                                })?;
                            let (int_part, rest) = s.split_once('.').unwrap_or((s, ""));
                            let (frac_part, exp_part) = rest
                                .split_once('p')
                                .or_else(|| rest.split_once('P'))
                                .unwrap_or((rest, ""));
                            let int_val = i64::from_str_radix(int_part, 16).unwrap_or(0);
                            let frac_val = if !frac_part.is_empty() {
                                let frac_bits = i64::from_str_radix(frac_part, 16).unwrap_or(0);
                                let frac_len = frac_part.len() as u32;
                                frac_bits as f64 / (16u64.pow(frac_len) as f64)
                            } else {
                                0.0
                            };
                            let exp: i32 = if !exp_part.is_empty() {
                                exp_part.parse().map_err(|_| {
                                    PyError::value_error(format!(
                                        "invalid hex float exponent: {}",
                                        exp_part
                                    ))
                                })?
                            } else {
                                0
                            };
                            let significand = int_val as f64 + frac_val;
                            let result = sign * significand * (2.0f64).powi(exp);
                            Ok(py_float(result))
                        },
                    }));
                }
                if bf_name == "float" && name == "hex" {
                    return Ok(PyObjectRef::imm(PyObject::BuiltinFunction {
                        name: "hex".to_string(),
                        func: |args| {
                            if args.is_empty() {
                                return Err(PyError::type_error("hex() takes exactly 1 argument"));
                            }
                            let obj = args[0].borrow();
                            if let PyObject::Float(v) = &*obj {
                                let bits = v.to_bits();
                                let sign = if (bits >> 63) != 0 { "-" } else { "" };
                                let biased_exp = ((bits >> 52) & 0x7ff) as i64;
                                let mantissa = bits & 0x000f_ffff_ffff_ffff;
                                if biased_exp == 0x7ff {
                                    if mantissa == 0 {
                                        Ok(py_str(&format!("{}inf", sign)))
                                    } else {
                                        Ok(py_str(&format!("{}nan", sign)))
                                    }
                                } else if *v == 0.0 {
                                    Ok(py_str(&format!("{}0x0.0p+0", sign)))
                                } else {
                                    let exp = biased_exp - 1023;
                                    let hex_mantissa = format!("{:013x}", mantissa);
                                    let hex_mantissa = hex_mantissa.trim_end_matches('0');
                                    Ok(py_str(&format!(
                                        "{}0x1.{}p{:+}",
                                        sign,
                                        if hex_mantissa.is_empty() {
                                            "0"
                                        } else {
                                            hex_mantissa
                                        },
                                        exp
                                    )))
                                }
                            } else {
                                Err(PyError::type_error("hex() argument must be float"))
                            }
                        },
                    }));
                }
                if bf_name == "float" && name == "from_number" {
                    return Ok(PyObjectRef::imm(PyObject::BuiltinFunction {
                        name: "from_number".to_string(),
                        func: |args| {
                            if args.is_empty() {
                                return Err(PyError::type_error(
                                    "float.from_number() takes exactly 1 argument",
                                ));
                            }
                            Ok(py_float(args[0].as_f64().unwrap_or(f64::NAN)))
                        },
                    }));
                }
                if bf_name == "int" && name == "from_bytes" {
                    return Ok(PyObjectRef::imm(PyObject::BuiltinFunction {
                        name: "from_bytes".to_string(),
                        func: builtin_int_from_bytes,
                    }));
                }
                if bf_name == "dict" && name == "fromkeys" {
                    // dict.fromkeys(iterable, value=None) — a real classmethod
                    // in CPython, called both as `dict.fromkeys(...)` and via
                    // `cls.fromkeys(...)` inside a dict-subclass's own
                    // methods (real code: `collections.ChainMap.__iter__`
                    // does `dict.fromkeys(mapping)`). Missing entirely before
                    // — `dict` has no attribute dict of its own to answer
                    // this from, being a plain BuiltinFunction constructor
                    // rather than a real Type.
                    return Ok(PyObjectRef::imm(PyObject::BuiltinFunction {
                        name: "fromkeys".to_string(),
                        func: |args| {
                            if args.is_empty() {
                                return Err(PyError::type_error(
                                    "fromkeys() takes at least 1 argument",
                                ));
                            }
                            let keys = crate::object::collect_iterable(&args[0])?;
                            let value = args.get(1).cloned().unwrap_or_else(py_none);
                            let mut d = PyDict::new();
                            for k in keys {
                                d.set(k, value.clone())?;
                            }
                            Ok(PyObjectRef::new(PyObject::Dict(Box::new(d))))
                        },
                    }));
                }
                if bf_name == "dict" && (name == "__setitem__" || name == "__getitem__") {
                    let method_name = name.to_string();
                    return Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: method_name.clone(),
                        func: if method_name == "__setitem__" {
                            builtin_dict_setitem as BuiltinFunc
                        } else {
                            builtin_dict_getitem as BuiltinFunc
                        },
                        self_obj: py_none(),
                    }));
                }
                // Built-in types (int, str, list, dict, ...) are represented
                // as a plain callable BuiltinFunction here, not a real class
                // object with its own bases/mro — so `int.mro()`-style
                // introspection (used e.g. by Django's lazy() for wrapping
                // arbitrary result types) has nothing real to walk. Returning
                // just [self] is not a correct ancestor chain (misses
                // `object`, and any real base for exception types etc.), but
                // it lets that code iterate something instead of crashing.
                if name == "mro" {
                    return Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "mro".to_string(),
                        func: |args| Ok(py_list(vec![args[0].clone()])),
                        self_obj: py_none(),
                    }));
                }
                if name == "__name__" {
                    return Ok(py_str(bf_name));
                }
                if name == "__qualname__" {
                    return Ok(py_str(bf_name));
                }
                // Same gap, same fix, as the real `PyObject::Type`'s own
                // `__module__` fallback just above — this is the OTHER
                // ad-hoc-type representation (built-in exception "classes"),
                // which need it too (e.g. `Exception.__module__`).
                if name == "__module__" {
                    return Ok(py_str("builtins"));
                }
                if name == "__mro__" || name == "__bases__" {
                    return Ok(PyObjectRef::new(PyObject::Tuple(vec![])));
                }
                if name == "__dict__" {
                    return Ok(PyObjectRef::new(PyObject::Dict(Box::new(PyDict::new()))));
                }
                if bf_name == "bool" && name == "__new__" {
                    return Ok(PyObjectRef::imm(PyObject::BuiltinFunction {
                        name: "__new__".to_string(),
                        func: |args| {
                            if args.is_empty() {
                                return Ok(py_bool(false));
                            }
                            if args.len() >= 2 {
                                return Ok(py_bool(args[1].truthy()));
                            }
                            Ok(py_bool(false))
                        },
                    }));
                }
                // A handful of generic dunders every real builtin function/
                // type has in CPython, regardless of which specific one —
                // were missing across the board (not one-off gaps), so
                // adding them here (rather than per-name like `fromhex`/
                // `__getformat__` above) covers `int`/`str`/`list`/`dict`/
                // any other native constructor uniformly. Real trigger:
                // CPython's own `test_heapq.py` (`__module__`), `test_call.py`/
                // `test_structseq.py` (`__new__`/`__init__` — common
                // "is this constructible via type.__new__" introspection),
                // `test_complex.py` (`__hash__` — checking hashability).
                if name == "__module__" {
                    return Ok(py_str("builtins"));
                }
                if name == "__hash__" {
                    return Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__hash__".to_string(),
                        func: |args| Ok(py_int(args[0].hash()? as i64)),
                        self_obj: py_none(),
                    }));
                }
                // `__reduce_ex__`/`__reduce__` — every object inherits these
                // from `object` in real CPython, but this native
                // `BuiltinFunction` representation (used for both actual
                // builtin functions like `abs`/`len` AND for builtin TYPES
                // like `int`/`str`/`dict`, which are represented as a plain
                // callable rather than a real `Type`) had no fallback at
                // all, unconditionally raising `AttributeError`.
                // `Lib/pickle.py`'s own `Pickler.save()` calls
                // `getattr(obj, "__reduce_ex__", _NoValue)` for anything
                // without a dispatch-table entry — missing this made EVERY
                // attempt to pickle a builtin function/method (or a builtin
                // type not otherwise special-cased) raise `PicklingError`,
                // general and widespread, not test-specific (confirmed via
                // `test_pickle.py`'s `test_builtin_functions`/
                // `test_builtin_exceptions`, which iterate literally every
                // entry in `builtins.__dict__`). Real CPython's own default
                // `__reduce_ex__` for a plain global function/type just
                // returns its qualified NAME as a string, which
                // `pickle.py` special-cases as "save by global reference"
                // (`if isinstance(rv, str): self.save_global(obj, rv)`) —
                // so returning the name string here is the correct, minimal
                // implementation, not a stand-in.
                if name == "__reduce_ex__" || name == "__reduce__" {
                    let qualname = bf_name.clone();
                    return Ok(PyObjectRef::new(PyObject::Closure(Rc::new(
                        move |_args: &[PyObjectRef]| Ok(py_str(&qualname)),
                    ))));
                }
                if name == "__new__" || name == "__init__" {
                    // CPython: `int.__new__(bool, ...)` raises TypeError
                    // ("int.__new__(bool) is not safe, use bool.__new__()")
                    // — bool has its own allocator. test_bool::test_subclass.
                    if name == "__new__" && bf_name == "int" {
                        return Ok(PyObjectRef::imm(PyObject::BuiltinFunction {
                            name: "int.__new__".to_string(),
                            func: int_new_checked,
                        }));
                    }
                    // Pragmatic stand-in: real CPython's builtin `__new__`/
                    // `__init__` slots are the actual C-level allocators/
                    // initializers, not separately-callable Python-visible
                    // functions with independent behavior worth
                    // reimplementing here — returning the constructor
                    // itself is "good enough" for introspection code that
                    // just checks these exist/are callable (real trigger:
                    // `test_structseq.py`'s `SomeStructType.__new__`-based
                    // construction pattern) without claiming to model the
                    // real two-phase alloc/init protocol.
                    return Ok(PyObjectRef::imm(PyObject::BuiltinFunction {
                        name: bf_name.clone(),
                        func: *func,
                    }));
                }
                Err(PyError::attribute_error(format!(
                    "'{}' object has no attribute '{}'",
                    o.type_name(),
                    name
                )))
            }
            PyObject::Code(c) => {
                match name {
                    "co_filename" => Ok(py_str(crate::interner::lookup_str(c.filename))),
                    "co_name" => Ok(py_str(crate::interner::lookup_str(c.name))),
                    "co_argcount" => Ok(py_int(c.arg_count as i64)),
                    "co_nlocals" => Ok(py_int(c.nlocals as i64)),
                    "co_varnames" => Ok(py_tuple(
                        c.varnames
                            .iter()
                            .map(|&v| py_str(crate::interner::lookup_str(v)))
                            .collect(),
                    )),
                    "co_flags" => Ok(py_int(c.flags as i64)),
                    // A handful of other commonly-introspected `co_*`
                    // fields were missing entirely (`AttributeError`) —
                    // real trigger: CPython's own `test_super.py`'s direct
                    // `func.__code__.co_firstlineno` check, among others.
                    "co_firstlineno" => Ok(py_int(c.first_lineno as i64)),
                    "co_kwonlyargcount" => Ok(py_int(c.kwonlyarg_count as i64)),
                    "co_posonlyargcount" => Ok(py_int(c.posonlyarg_count as i64)),
                    "co_names" => Ok(py_tuple(
                        c.names
                            .iter()
                            .map(|&v| py_str(crate::interner::lookup_str(v)))
                            .collect(),
                    )),
                    "co_consts" => Ok(py_tuple(
                        c.consts
                            .iter()
                            .filter_map(|cv| crate::vm::eval_const_value(cv.clone()).ok())
                            .collect(),
                    )),
                    "co_cellvars" => Ok(py_tuple(
                        c.cellvars.iter().map(|v| py_str(v)).collect(),
                    )),
                    "co_freevars" => Ok(py_tuple(
                        c.freevars.iter().map(|v| py_str(v)).collect(),
                    )),
                    // `types.CodeType.replace(**changes)` — returns a copy of
                    // the code object with the given fields overridden.
                    // Missing entirely before (AttributeError), breaking any
                    // introspection/rewriting code that walks and rebuilds
                    // `__code__` (test_listcomps.py's `test_code_replace`,
                    // which recursively rewrites `co_consts` for nested code
                    // objects). `BuiltinMethod`'s calling convention packs
                    // kwargs as a trailing dict (see CLAUDE.md gotchas), so
                    // this only ever receives `[self, kwargs_dict?]`.
                    "replace" => {
                        let code_rc = c.clone();
                        Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                            name: "replace".to_string(),
                            func: code_replace,
                            self_obj: PyObjectRef::new(PyObject::Code(code_rc)),
                        }))
                    }
                    "co_qualname" => Ok(py_str(crate::interner::lookup_str(c.qualname))),
                    // `co_lines()` — a *method* (not a plain attribute)
                    // since CPython 3.10 replacing the old `co_lnotab`
                    // bytes blob: returns an iterator of
                    // `(start_byte, end_byte, line)` triples over
                    // `co_code`. Missing entirely before (`AttributeError`
                    // on every call — real trigger: `test_compile.py`
                    // reading `[line for _, _, line in co.co_lines()]`).
                    // Byte offsets here use the same "2 bytes per
                    // instruction" convention as the `co_code` synthesis
                    // below so the two stay consistent; consecutive
                    // instructions sharing a line number are coalesced
                    // into a single triple, matching real CPython's
                    // behavior of one entry per contiguous same-line run
                    // rather than one entry per instruction.
                    "co_lines" => {
                        let lines = c.line_numbers.clone();
                        Ok(PyObjectRef::new(PyObject::Closure(Rc::new(
                            move |_args: &[PyObjectRef]| {
                                let mut triples = Vec::new();
                                let mut i = 0usize;
                                while i < lines.len() {
                                    let line = lines[i];
                                    let start = i;
                                    let mut j = i + 1;
                                    while j < lines.len() && lines[j] == line {
                                        j += 1;
                                    }
                                    if line != 0 {
                                        triples.push(py_tuple(vec![
                                            py_int((start * 2) as i64),
                                            py_int((j * 2) as i64),
                                            py_int(line as i64),
                                        ]));
                                    }
                                    i = j;
                                }
                                Ok(py_list(triples))
                            },
                        ))))
                    }
                    // This VM's bytecode isn't CPython's actual wordcode
                    // format, so there's no real "true" `co_code` bytes to
                    // return — but real code (`test_compile.py`'s
                    // `co_lines()`/dis-adjacent introspection, `dis.dis`
                    // fallbacks, etc.) just needs *some* `bytes` object of
                    // a length consistent with `co_lines()`'s reported byte
                    // ranges, not byte-identical CPython opcodes. Each of
                    // this VM's instructions is represented as one 2-byte
                    // (opcode, arg-low-byte) pair, matching CPython
                    // 3.11+'s fixed 2-byte instruction width, so
                    // `len(co_code) == 2 * len(instructions)` stays
                    // internally consistent with the offsets `co_lines()`
                    // hands out below.
                    "co_code" => {
                        let mut bytes = Vec::with_capacity(c.instructions.len() * 2);
                        for instr in &c.instructions {
                            bytes.push(instr.op as u8);
                            bytes.push((instr.arg & 0xff) as u8);
                        }
                        Ok(PyObjectRef::imm(PyObject::Bytes(bytes)))
                    }
                    // Best-effort operand-stack depth estimate: this VM
                    // doesn't compute or enforce a fixed max stack size
                    // (its stack is just a growable `Vec`), so there was no
                    // real value to report here at all — real trigger:
                    // `test_compile.py` reading `code.co_stacksize`
                    // directly, `AttributeError` on every access. Walk the
                    // instructions with a coarse per-opcode net stack
                    // effect (most opcodes: push 1 and pop `nargs`-ish;
                    // unrecognized/control-flow opcodes assumed net-zero)
                    // and report the running peak, floored at 1 — approximate,
                    // not CPython-bit-exact, but a real, non-crashing value
                    // in the right ballpark rather than a hardcoded constant.
                    "co_stacksize" => {
                        let mut depth: i64 = 0;
                        let mut peak: i64 = 1;
                        for instr in c.instructions.iter() {
                            depth += crate::bytecode::stack_effect(instr.op, instr.arg);
                            if depth < 0 {
                                depth = 0;
                            }
                            if depth > peak {
                                peak = depth;
                            }
                        }
                        Ok(py_int(peak))
                    }
                    _ => Err(PyError::attribute_error(format!(
                        "'code' object has no attribute '{}'",
                        name
                    ))),
                }
            }
            PyObject::BuiltinMethod { name: bm_name, func, self_obj } => match name {
                "__self__" => Ok(self_obj.clone()),
                "__func__" => Ok(PyObjectRef::new(PyObject::BuiltinFunction {
                    name: bm_name.clone(),
                    func: *func,
                })),
                "__name__" => Ok(py_str(bm_name)),
                "__qualname__" => Ok(py_str(bm_name)),
                "__module__" => Ok(py_str("builtins")),
                "__doc__" => Ok(py_none()),
                // Same gap, same fix, as `BuiltinFunction`'s own
                // `__reduce_ex__`/`__reduce__` fallback above (a bound
                // native method, e.g. `[].append`, needs the same "reduce
                // by name" default `object` behavior).
                "__reduce_ex__" | "__reduce__" => {
                    let qualname = bm_name.clone();
                    Ok(PyObjectRef::new(PyObject::Closure(Rc::new(
                        move |_args: &[PyObjectRef]| Ok(py_str(&qualname)),
                    ))))
                }
                _ => Err(PyError::attribute_error(format!(
                    "'builtin_function_or_method' object has no attribute '{}'",
                    name
                ))),
            },
        _ => unreachable!("{} called with non-{} object", stringify!(get), o.type_name()),
    }
}
