// Split out of the former monolithic object.rs (see the file-splitting
// refactor's memory entry for context) — this file holds the ~79 standalone
// `pub fn builtin_*` free functions (the builtins module's actual
// implementations: print, len, isinstance, issubclass, format, iter/next,
// eval/exec, and so on).
use super::*;

// ---- Built-in functions ----

// Fallback only — real dispatch goes through `print_with_vm` via a
// `fn_addr_eq` special-case in `vm.rs`'s `call_function` (see
// `print_with_vm`'s own doc comment for why this needs the live VM).
pub fn builtin_print(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    with_vm_mut(|vm| print_with_vm(vm, args, &[]))?
}

/// The real `print()` implementation — needs a live `&mut VirtualMachine` to
/// look up the CURRENT value of `sys.stdout` (not a cached reference) so
/// `contextlib.redirect_stdout`/`unittest.mock.patch('sys.stdout', ...)`-style
/// substitution actually takes effect, and to call arbitrary objects'
/// `write`/`flush` methods (a plain `io::stdout()` `println!()`, which is
/// what this used to be, can reach neither). Previously this also silently
/// ignored `sep`/`end`/`file`/`flush` keyword arguments entirely — they were
/// packed into a trailing dict (this project's established kwargs-passing
/// convention for plain `BuiltinFunction`s) and then that dict got PRINTED
/// AS A POSITIONAL ARGUMENT, since the old code just joined every element of
/// `args` unconditionally. Confirmed via the simplest possible repro:
/// `print("x", end="")` printed `x {'end': ''}` instead of `x` with no
/// trailing newline. Given how extremely common `sep=`/`end=`/`file=` and
/// stdout-capturing test patterns both are in real Python code, this was one
/// of the most broadly-impactful gaps found this session.
pub(crate) fn print_with_vm(vm: &mut crate::vm::VirtualMachine, args: &[PyObjectRef], keywords: &[(String, PyObjectRef)]) -> PyResult<PyObjectRef> {
    let mut sep = " ".to_string();
    let mut end = "\n".to_string();
    let mut file: Option<PyObjectRef> = None;
    let mut flush = false;
    for (k, v) in keywords {
        match k.as_str() {
            "sep" => {
                if !matches!(&*v.borrow(), PyObject::None) { sep = v.str(); }
            }
            "end" => {
                if !matches!(&*v.borrow(), PyObject::None) { end = v.str(); }
            }
            "file" => {
                if !matches!(&*v.borrow(), PyObject::None) { file = Some(v.clone()); }
            }
            "flush" => { flush = v.truthy(); }
            _ => {}
        }
    }

    let strings: Vec<String> = args.iter().map(|a| a.str()).collect();
    let mut output = strings.join(&sep);
    output.push_str(&end);

    let target = match file {
        Some(f) => f,
        None => vm.modules.get("sys")
            .and_then(|m| if let PyObject::Module { dict, .. } = &*m.borrow() { dict.get_str("stdout").cloned() } else { None })
            .ok_or_else(|| PyError::runtime_error("lost sys.stdout"))?,
    };

    call_method_rebound(vm, &target, "write", vec![py_str(&output)])
        .map_err(|_| PyError::attribute_error("'file' object has no attribute 'write'"))?;

    if flush {
        let _ = call_method_rebound(vm, &target, "flush", vec![]);
    }

    Ok(py_none())
}

/// Calls `target.<name>(call_args...)`, rebinding a native `BuiltinMethod`'s
/// `self_obj` to `target` directly (ONE prepended self, matching how
/// `LOAD_ATTR` itself rebinds container methods like `File`/`List`/`Dict`'s
/// `write`/`append`/etc. for ordinary dot-call syntax) — NOT
/// `call_bound_method`'s convention, which prepends BOTH the method's own
/// (placeholder) `self_obj` AND an explicit second one, meant for dunder
/// methods that are written expecting that double-self shape. Using
/// `call_bound_method` here initially caused `File::write`'s own `args[0]`
/// check to see the leftover placeholder instead of the real file, raising
/// "write on non-file" — confirmed by testing plain `f.write(x)` (which
/// goes through `LOAD_ATTR`'s rebind-in-place logic, not `call_bound_method`)
/// working correctly on the exact same object.
fn call_method_rebound(vm: &mut crate::vm::VirtualMachine, target: &PyObjectRef, name: &str, call_args: Vec<PyObjectRef>) -> PyResult<PyObjectRef> {
    let method = target.borrow().get_attribute(name)?;
    let bound = match &*method.borrow() {
        PyObject::BuiltinMethod { func, name: mname, .. } => {
            PyObjectRef::imm(PyObject::BuiltinMethod { name: mname.clone(), func: *func, self_obj: target.clone() })
        }
        _ => method.clone(),
    };
    vm.call_function(bound, call_args, vec![])
}

pub fn builtin_len(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() != 1 {
        return Err(PyError::type_error("len() takes exactly one argument"));
    }
    let obj = args[0].borrow();
    match &*obj {
        PyObject::Str(s) => Ok(py_int(s.chars().count())),
        PyObject::List(v) => Ok(py_int(v.len())),
        PyObject::Tuple(v) => Ok(py_int(v.len())),
        PyObject::Dict(d) => Ok(py_int(d.len())),
        PyObject::Set(s) => Ok(py_int(s.len())),
        PyObject::FrozenSet(s) => Ok(py_int(s.len())),
        PyObject::Range { start, stop, step } => {
            if *step > 0 && *start >= *stop { Ok(py_int(0)) }
            else if *step < 0 && *start <= *stop { Ok(py_int(0)) }
            else {
                let raw_len = stop.checked_sub(*start).unwrap_or(i64::MAX);
                let len = raw_len.checked_div(*step).unwrap_or(0) as i64;
                if raw_len % *step != 0 { Ok(py_int(len.abs() + 1)) }
                else { Ok(py_int(len.abs())) }
            }
        }
        PyObject::Bytes(b) => Ok(py_int(b.len())),
        PyObject::ByteArray(b) => Ok(py_int(b.len())),
        PyObject::Array(arr) => Ok(py_int(arr.data.len())),
        PyObject::Instance { typ, dict } => {
            let f = lookup_dunder_via_mro(typ, "__len__");
            if let Some(f) = f {
                let result = call_bound_method(f, args[0].clone(), vec![])?;
                let n = result.borrow();
                if let PyObject::Int(i) = &*n {
                    // Real CPython rejects a negative `__len__()` result
                    // with `ValueError: __len__() should return >= 0` —
                    // this was missing entirely, silently accepting -1 as
                    // a length. Confirmed via CPython's own
                    // `test_bool.test_sane_len`, which asserts `bool()`'s
                    // and `len()`'s error messages for the same bad
                    // `__len__` values are identical — `bool()` delegates
                    // to this same function specifically so that holds.
                    if i.sign() == Sign::Minus {
                        return Err(PyError::value_error("__len__() should return >= 0"));
                    }
                    return Ok(py_int(i.clone()));
                }
                return Err(PyError::type_error("__len__() should return an int"))
            }
            if let Some(native) = dict.get(NATIVE_BACKING_KEY) {
                return builtin_len(&[native.clone()]);
            }
            Err(PyError::type_error(format!("object of type '{}' has no len()", obj.type_name())))
        }
        // A class object itself, via its metaclass's `__len__` (e.g.
        // `len(SomeEnum)` — see the matching GET_ITER/builtin_iter handling
        // for why this needs metatype_of rather than ordinary lookup).
        PyObject::Type { .. } => {
            let f = metatype_of(&args[0]).and_then(|mt| lookup_dunder_via_mro(&mt, "__len__"));
            if let Some(f) = f {
                let result = call_bound_method(f, args[0].clone(), vec![])?;
                let n = result.borrow();
                if let PyObject::Int(i) = &*n {
                    if i.sign() == Sign::Minus {
                        return Err(PyError::value_error("__len__() should return >= 0"));
                    }
                    return Ok(py_int(i.clone()));
                }
                return Err(PyError::type_error("__len__() should return an int"));
            }
            Err(PyError::type_error(format!("object of type '{}' has no len()", obj.type_name())))
        }
        _ => Err(PyError::type_error(format!("object of type '{}' has no len()", obj.type_name()))),
    }
}

/// Cheap, best-effort size hint for materializing an arbitrary iterable
/// into a `Vec` (used by `list()`/`tuple()`). Real CPython pre-sizes via
/// `PyObject_LengthHint` before iterating, so a source with an O(1) `len()`
/// (e.g. `range(huge)`) fails fast with a single allocation attempt instead
/// of growing the backing buffer one doubling at a time — which, for
/// something like `list(range(sys.maxsize // 2))`, can consume the
/// system's entire RAM over many reallocations before ever failing (each
/// individual `push()` succeeds right up until physical memory runs out).
/// Returns `None` (not an error) when the object has no usable `__len__` —
/// callers should just skip pre-reservation and fall back to incremental
/// growth, which is fine for ordinary bounded iterables/generators.
fn iterable_length_hint(obj: &PyObjectRef) -> Option<usize> {
    let len_obj = builtin_len(std::slice::from_ref(obj)).ok()?;
    let borrowed = len_obj.borrow();
    match &*borrowed {
        PyObject::Int(n) => n.to_usize(),
        _ => None,
    }
}

pub fn builtin_range(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    match args.len() {
        1 => {
            let stop = args[0].borrow();
            if let PyObject::Int(n) = &*stop {
                let stop = n.to_i64().ok_or_else(|| PyError::type_error("range() expects int arguments"))?;
                Ok(PyObjectRef::imm(PyObject::Range { start: 0, stop, step: 1 }))
            } else {
                Err(PyError::type_error("range() expects int arguments"))
            }
        }
        2 => {
            let start = args[0].borrow();
            let stop = args[1].borrow();
            if let (PyObject::Int(a), PyObject::Int(b)) = (&*start, &*stop) {
                let a = a.to_i64().ok_or_else(|| PyError::type_error("range() expects int arguments"))?;
                let b = b.to_i64().ok_or_else(|| PyError::type_error("range() expects int arguments"))?;
                Ok(PyObjectRef::imm(PyObject::Range { start: a, stop: b, step: 1 }))
            } else {
                Err(PyError::type_error("range() expects int arguments"))
            }
        }
        3 => {
            let start = args[0].borrow();
            let stop = args[1].borrow();
            let step = args[2].borrow();
            if let (PyObject::Int(a), PyObject::Int(b), PyObject::Int(s)) = (&*start, &*stop, &*step) {
                let a = a.to_i64().ok_or_else(|| PyError::type_error("range() expects int arguments"))?;
                let b = b.to_i64().ok_or_else(|| PyError::type_error("range() expects int arguments"))?;
                let s = s.to_i64().ok_or_else(|| PyError::type_error("range() expects int arguments"))?;
                if s == 0 { return Err(PyError::value_error("range() arg 3 must not be zero")); }
                Ok(PyObjectRef::imm(PyObject::Range { start: a, stop: b, step: s }))
            } else {
                Err(PyError::type_error("range() expects int arguments"))
            }
        }
        _ => Err(PyError::type_error("range() takes at most 3 arguments")),
    }
}

thread_local! {
    // `type(x)` for a builtin-native value (int/str/list/...) used to build
    // a BRAND NEW, throwaway `PyObject::Type` on every single call — so
    // `type(5) is type(6)` (and even `type(5) is type(5)`, two separate
    // calls) was ALWAYS `False`, since no two calls ever returned the same
    // object. This is an extremely common idiom (`type(self) is type(other)`
    // total-ordering-style guards, `type(x) == int` checks) — confirmed via
    // CPython's own `test_math.testIsqrt`'s `self.assertIs(type(s), int)`.
    // Caching one canonical Type object per builtin type NAME here fixes
    // same-kind identity comparisons. For a type that has been migrated to
    // a REAL `PyObject::Type` registered in `builtins` (see
    // `NATIVE_VALUE_CTOR_KEY`'s doc comment — `int` as of this writing),
    // `seed_primitive_type_cache` below pre-populates this cache with that
    // SAME canonical object at `create_builtins()` time, so `type(5) is
    // int` is genuinely `True` — not just `type(5) is type(5)`. For any
    // type NOT yet migrated, this cache still falls back to lazily
    // building a fresh placeholder `Type` per name on first use, exactly
    // as before.
    static PRIMITIVE_TYPE_CACHE: std::cell::RefCell<HashMap<String, PyObjectRef>> = std::cell::RefCell::new(HashMap::new());
}

/// Pre-seed `PRIMITIVE_TYPE_CACHE` with the canonical, already-constructed
/// `Type` object for a native value type (called once from
/// `create_builtins()` right after building e.g. `int_type`) — so
/// `builtin_type_of`/`type(x)` returns this SAME object instead of lazily
/// building an unrelated placeholder the first time `type(5)` is called.
pub(crate) fn seed_primitive_type_cache(name: &str, ty: PyObjectRef) {
    PRIMITIVE_TYPE_CACHE.with(|c| { c.borrow_mut().insert(name.to_string(), ty); });
}

pub fn builtin_type_of(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() == 1 {
        // type(obj) -> return the type of an object
        let borrowed = args[0].borrow();
        match &*borrowed {
            PyObject::Instance { typ, .. } => Ok(typ.clone()),
            PyObject::Type { .. } => Ok(args[0].clone()),
            _ => {
                let name = borrowed.type_name();
                drop(borrowed);
                if let Some(cached) = PRIMITIVE_TYPE_CACHE.with(|c| c.borrow().get(&name).cloned()) {
                    return Ok(cached);
                }
                let new_type = PyObjectRef::new(PyObject::Type {
                    name: name.clone(),
                    dict: Box::new(TypeDict::default()),
                    bases: vec![],
                    mro: vec![],
                });
                PRIMITIVE_TYPE_CACHE.with(|c| { c.borrow_mut().insert(name, new_type.clone()); });
                Ok(new_type)
            }
        }
    } else if args.len() == 3 {
        // type(name, bases, dict) -> create a new class (metaclass usage).
        // Delegates to the VM's default_build_class so a dynamically
        // created class gets exactly the same treatment as one from a
        // `class Foo(...):` statement (native-base propagation, real C3
        // MRO, __set_name__, __init_subclass__) instead of the separate,
        // less complete hand-rolled logic this used to have.
        let bases_vec = to_bases_vec(&args[1]);
        let namespace_dict = dict_arg_to_hashmap(&args[2], "type() third argument must be a dict")?;
        with_vm_mut(|vm| vm.default_build_class(args[0].str(), bases_vec, namespace_dict, vec![], None))?
    } else {
        Err(PyError::type_error("type() takes exactly one or three arguments"))
    }
}

fn to_bases_vec(bases: &PyObjectRef) -> Vec<PyObjectRef> {
    if let PyObject::Tuple(t) = &*bases.borrow() {
        t.clone()
    } else if matches!(&*bases.borrow(), PyObject::None) {
        vec![]
    } else {
        vec![bases.clone()]
    }
}

/// A class-namespace argument (`type.__new__`'s 4th positional arg, or
/// `type(name, bases, ns)`'s 3rd) is usually a plain dict, but when a
/// metaclass has a `__prepare__` returning a real dict-subclass instance
/// (e.g. enum's `_EnumDict`, used to track member-definition order via an
/// overridden `__setitem__` — see `EnumType.__prepare__`), it arrives here
/// as a `PyObject::Instance` whose actual dict contents live in its native
/// backing, not a bare `PyObject::Dict`. Check both.
pub(crate) fn dict_arg_to_hashmap(namespace: &PyObjectRef, err_msg: &str) -> PyResult<HashMap<String, PyObjectRef>> {
    if let Some(native) = native_backing_of(namespace) {
        return dict_arg_to_hashmap(&native, err_msg);
    }
    match &*namespace.borrow() {
        PyObject::Dict(d) => Ok(d.items().into_iter().map(|(k, v)| (k.str(), v)).collect()),
        _ => Err(PyError::type_error(err_msg)),
    }
}

/// `type.__new__(metacls, name, bases, namespace, **kwds)` — the real,
/// CPython-shaped 4-argument metaclass `__new__` convention (distinct from
/// `builtin_type_of`'s `type(x)`/`type(name, bases, ns)` conventions above,
/// which have no `metacls` parameter — kept as two separate functions so
/// the two calling shapes are never ambiguous). Reached when a user
/// metaclass's own `__new__` calls `super().__new__(metacls, name, bases,
/// namespace, **kwds)` and the super-mro walk bottoms out at plain `type`
/// (see `type`'s registration in `create_builtins`).
pub fn type_new_builtin(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if std::env::var("RPY_DEBUG_METACLASS").is_ok() {
        eprintln!("type_new_builtin: args.len()={} args={:?}", args.len(), args.iter().map(|a| a.repr()).collect::<Vec<_>>());
    }
    if args.len() < 4 {
        return Err(PyError::type_error("type.__new__() takes at least 4 arguments (metacls, name, bases, namespace)"));
    }
    let metacls = args[0].clone();
    let name_str = args[1].str();
    let bases_vec = to_bases_vec(&args[2]);
    let namespace_dict = dict_arg_to_hashmap(&args[3], "type.__new__(): namespace must be a dict")?;
    let kwargs: Vec<(String, PyObjectRef)> = args.get(4)
        .map(|d| dict_arg_to_hashmap(d, "").unwrap_or_default().into_iter().collect())
        .unwrap_or_default();
    let metatype = with_vm_mut(|vm| {
        let is_bare_type = vm.builtins.get(&interner::intern("type")).map(|t| t.is(&metacls)).unwrap_or(false);
        if is_bare_type { None } else { Some(metacls.clone()) }
    })?;
    with_vm_mut(|vm| vm.default_build_class(name_str, bases_vec, namespace_dict, kwargs, metatype))?
}

pub fn builtin_int(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() { return Ok(py_int(0)); }
    let obj = args[0].borrow();
    match &*obj {
        PyObject::Int(_) => Ok(args[0].clone()),
        PyObject::Float(f) => Ok(py_int(*f as i64)),
        PyObject::Str(s) => {
            let s_trim = s.trim();
            #[cfg(not(feature = "no_int_str_limit"))]
            if args.len() < 2 {
                let limit = INT_MAX_STR_DIGITS.with(|d| d.get());
                if limit > 0 {
                    let digit_len = s_trim.trim_start_matches(|c: char| c == '+' || c == '-').len();
                    if digit_len > limit as usize {
                        return Err(PyError::value_error(format!(
                            "Exceeds the limit ({} digits) for integer string conversion; use sys.set_int_max_str_digits()", limit
                        )));
                    }
                }
            }
            // Remove underscores (Python visual separator, e.g. "0xFF_FF" or "1_000_000")
            let s_clean: String = s_trim.chars().filter(|&c| c != '_').collect();
            // Split optional sign from body
            let (sign, body) = match s_clean.as_bytes().first() {
                Some(b'-') => (-1, &s_clean[1..]),
                Some(b'+') => (1, &s_clean[1..]),
                _ => (1, &s_clean[..]),
            };
            let make_err = || PyError::value_error(format!("invalid literal for int(): '{}'", s));
            let obj = if let Some(oct) = body.strip_prefix("0o").or_else(|| body.strip_prefix("0O")) {
                if let Ok(n) = i64::from_str_radix(oct, 8) { py_int(sign * n) }
                else if let Some(n) = BigInt::parse_bytes(oct.as_bytes(), 8) { py_int(if sign < 0 { -n } else { n }) }
                else { return Err(make_err()); }
            } else if let Some(hex) = body.strip_prefix("0x").or_else(|| body.strip_prefix("0X")) {
                if let Ok(n) = i64::from_str_radix(hex, 16) { py_int(sign * n) }
                else if let Some(n) = BigInt::parse_bytes(hex.as_bytes(), 16) { py_int(if sign < 0 { -n } else { n }) }
                else { return Err(make_err()); }
            } else if let Some(bin) = body.strip_prefix("0b").or_else(|| body.strip_prefix("0B")) {
                if let Ok(n) = i64::from_str_radix(bin, 2) { py_int(sign * n) }
                else if let Some(n) = BigInt::parse_bytes(bin.as_bytes(), 2) { py_int(if sign < 0 { -n } else { n }) }
                else { return Err(make_err()); }
            } else if args.len() > 1 {
                // int(x, base): parse x in given base
                drop(obj);
                let base_val = args[1].borrow();
                let base = if let PyObject::Int(i) = &*base_val { i.to_i64().unwrap_or(10) as u32 }
                    else { return Err(PyError::type_error("int() base must be an integer")) };
                if base < 2 || base > 36 {
                    return Err(PyError::value_error("int() base must be >= 2 and <= 36"));
                }
                // Re-borrow the string
                let obj2 = args[0].borrow();
                if let PyObject::Str(s) = &*obj2 {
                    let s_trim = s.trim();
                    let s_clean: String = s_trim.chars().filter(|&c| c != '_').collect();
                    let (sign, body) = match s_clean.as_bytes().first() {
                        Some(b'-') => (-1, &s_clean[1..]),
                        Some(b'+') => (1, &s_clean[1..]),
                        _ => (1, &s_clean[..]),
                    };
                    if let Ok(n) = i64::from_str_radix(body, base) { return Ok(py_int(sign * n)); }
                    else if let Some(n) = BigInt::parse_bytes(body.as_bytes(), base) { return Ok(py_int(if sign < 0 { -n } else { n })); }
                    else { return Err(PyError::value_error(format!("invalid literal for int(): '{}'", s))); }
                } else {
                    return Err(PyError::type_error("int() can convert strings only with base"));
                }
            } else {
                if let Ok(n) = body.parse::<i64>() { py_int(sign * n) }
                else if let Ok(n) = body.parse::<BigInt>() { py_int(if sign < 0 { -n } else { n }) }
                else { return Err(make_err()); }
            };
            Ok(obj)
        }
        PyObject::Bool(b) => Ok(py_int(if *b { 1 } else { 0 })),
        PyObject::Instance { dict: _, typ: _ } => {
            drop(obj);
            // A class transparently subclassing `int` (e.g. IntEnum
            // members) with no `__int__` override converts via its native
            // backing directly — real Python's `int(x)` for an int
            // subclass instance just IS that underlying int value.
            if let Some(native) = native_backing_of(&args[0]) {
                return builtin_int(&[native]);
            }
            // Try calling __int__ method on the instance
            let args0 = &args[0];
            if let Ok(int_method) = args0.borrow().get_attribute("__int__") {
                let instance = args[0].clone();
                let result = builtin_call(&int_method, &[instance]);
                if let Ok(val) = result {
                    if let Some(n) = val.as_i64() {
                        return Ok(py_int(n));
                    }
                    // Maybe it returns a BigInt
                    let is_int = matches!(&*val.borrow(), PyObject::Int(_));
                    if is_int {
                        return Ok(val);
                    }
                }
            }
            Err(PyError::type_error(format!("int() argument must be a string or number, not '{}'", 
                args0.borrow().type_name())))
        }
        _ => Err(PyError::type_error(format!("int() argument must be a string or number, not '{}'", obj.type_name()))),
    }
}

/// int.from_bytes(bytes, byteorder, *, signed=False)
pub fn builtin_int_from_bytes(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 2 {
        return Err(PyError::type_error("int.from_bytes() needs at least 2 arguments"));
    }
    let bytes_val = &args[0];
    let byteorder = &args[1];
    let order_str = byteorder.str();
    let big_endian = order_str == "big";
    let byte_data: Vec<u8> = match &*bytes_val.borrow() {
        PyObject::Bytes(b) => b.clone(),
        PyObject::List(items) => {
            items.iter().map(|x| x.as_i64().unwrap_or(0) as u8).collect()
        }
        _ => {
            let mut v = Vec::new();
            if let Ok(it) = builtin_iter(&[bytes_val.clone()]) {
                loop {
                    match builtin_next(&[it.clone()]) {
                        Ok(x) => v.push(x.as_i64().unwrap_or(0) as u8),
                        Err(PyError::StopIteration) => break,
                        Err(e) => return Err(e),
                    }
                }
            }
            v
        }
    };
    let n = if big_endian {
        byte_data.iter().fold(0i64, |acc, &b| (acc << 8) | b as i64)
    } else {
        byte_data.iter().rev().fold(0i64, |acc, &b| (acc << 8) | b as i64)
    };
    Ok(py_int(n))
}

pub fn builtin_float(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() { return Ok(py_float(0.0)); }
    let obj = args[0].borrow();
    match &*obj {
        PyObject::Int(i) => Ok(py_float(i.to_f64().unwrap_or(0.0))),
        PyObject::Float(f) => Ok(py_float(*f)),
        PyObject::Str(s) => {
            let s: &str = s;
            let s = s.trim_matches(|c: char| c.is_whitespace());
            let normalized: String = s.chars().map(|c| {
                match c {
                    '\u{0660}'..='\u{0669}' => char::from_u32('0' as u32 + (c as u32 - 0x0660)).unwrap_or(c),
                    '\u{06F0}'..='\u{06F9}' => char::from_u32('0' as u32 + (c as u32 - 0x06F0)).unwrap_or(c),
                    '\u{0966}'..='\u{096F}' => char::from_u32('0' as u32 + (c as u32 - 0x0966)).unwrap_or(c),
                    _ => c,
                }
            }).collect();
            let f: f64 = normalized.parse().map_err(|_| PyError::value_error(format!("could not convert string to float: '{}'", s)))?;
            Ok(py_float(f))
        }
        PyObject::Bytes(b) => {
            let s = std::str::from_utf8(b).map_err(|_| PyError::value_error("could not convert bytes to float: invalid utf-8"))?;
            let s = s.trim_matches(|c: char| c.is_whitespace());
            let normalized: String = s.chars().map(|c| {
                match c {
                    '\u{0660}'..='\u{0669}' => char::from_u32('0' as u32 + (c as u32 - 0x0660)).unwrap_or(c),
                    '\u{06F0}'..='\u{06F9}' => char::from_u32('0' as u32 + (c as u32 - 0x06F0)).unwrap_or(c),
                    '\u{0966}'..='\u{096F}' => char::from_u32('0' as u32 + (c as u32 - 0x0966)).unwrap_or(c),
                    _ => c,
                }
            }).collect();
            let f: f64 = normalized.parse().map_err(|_| PyError::value_error(format!("could not convert string to float: '{}'", s)))?;
            Ok(py_float(f))
        }
        PyObject::ByteArray(b) => {
            let s = std::str::from_utf8(b).map_err(|_| PyError::value_error("could not convert bytearray to float: invalid utf-8"))?;
            let s = s.trim_matches(|c: char| c.is_whitespace());
            let normalized: String = s.chars().map(|c| {
                match c {
                    '\u{0660}'..='\u{0669}' => char::from_u32('0' as u32 + (c as u32 - 0x0660)).unwrap_or(c),
                    '\u{06F0}'..='\u{06F9}' => char::from_u32('0' as u32 + (c as u32 - 0x06F0)).unwrap_or(c),
                    '\u{0966}'..='\u{096F}' => char::from_u32('0' as u32 + (c as u32 - 0x0966)).unwrap_or(c),
                    _ => c,
                }
            }).collect();
            let f: f64 = normalized.parse().map_err(|_| PyError::value_error(format!("could not convert string to float: '{}'", s)))?;
            Ok(py_float(f))
        }
        PyObject::Instance { typ, .. } => {
            match lookup_dunder_via_mro(typ, "__float__") {
                Some(f) => {
                    drop(obj);
                    call_bound_method(f, args[0].clone(), vec![])
                }
                None => Err(PyError::type_error(format!("float() argument must be a string or number, not '{}'", get_type_name_for_instance(typ)))),
            }
        }
        _ => Err(PyError::type_error(format!("float() argument must be a string or number, not '{}'", obj.type_name()))),
    }
}

/// `float.fromhex(s)` — a genuine class-level-only method (called unbound,
/// `float.fromhex("0x1.8p3")`, never as `x.fromhex()` on a float instance),
/// extracted out of what used to be a `bf_name == "float" && name ==
/// "fromhex"` inline closure in `get_attribute_impl` (`attrs.rs`) so it can
/// live in `float`'s own type dict now that `float` is a real `Type` (see
/// `NATIVE_VALUE_CTOR_KEY`'s doc comment) — that string-name dispatch never
/// fires for a real `Type` object, only for the old bare `BuiltinFunction`
/// shape.
pub(crate) fn float_fromhex(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() { return Err(PyError::type_error("float.fromhex() requires exactly 1 argument")); }
    let s = args[0].str();
    let s = s.trim();
    let lower = s.to_lowercase();
    if lower == "nan" { return Ok(py_float(f64::NAN)); }
    if lower == "inf" || lower == "+inf" || lower == "-inf" || lower == "infinity" || lower == "+infinity" || lower == "-infinity" {
        let sign = if lower.starts_with('-') { -1.0 } else { 1.0 };
        return Ok(py_float(sign * f64::INFINITY));
    }
    let s = s.strip_prefix("+").unwrap_or(s);
    let sign = if s.starts_with('-') { -1.0 } else { 1.0 };
    let s = s.strip_prefix('-').unwrap_or(s.strip_prefix('+').unwrap_or(s));
    let s = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X"))
        .ok_or_else(|| PyError::value_error(format!("invalid hex float literal: {}", s)))?;
    let (int_part, rest) = s.split_once('.').unwrap_or((s, ""));
    let (frac_part, exp_part) = rest.split_once('p').or_else(|| rest.split_once('P'))
        .unwrap_or((rest, ""));
    let int_val = i64::from_str_radix(int_part, 16).unwrap_or(0);
    let frac_val = if !frac_part.is_empty() {
        let frac_bits = i64::from_str_radix(frac_part, 16).unwrap_or(0);
        let frac_len = frac_part.len() as u32;
        frac_bits as f64 / (16u64.pow(frac_len) as f64)
    } else { 0.0 };
    let exp: i32 = if !exp_part.is_empty() {
        exp_part.parse().map_err(|_| PyError::value_error(format!("invalid hex float exponent: {}", exp_part)))?
    } else { 0 };
    let significand = int_val as f64 + frac_val;
    let result = sign * significand * (2.0f64).powi(exp);
    Ok(py_float(result))
}

/// `float.hex(x)` — the unbound, explicit-argument class-level form (`float.
/// hex(3.5)`, as opposed to `x.hex()` on a float instance, which goes
/// through a wholly separate `PyObject::Float(_)` instance arm elsewhere in
/// `attrs.rs`, unaffected by this). Same extraction rationale as
/// `float_fromhex` above.
pub(crate) fn float_class_hex(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() { return Err(PyError::type_error("hex() takes exactly 1 argument")); }
    let obj = args[0].borrow();
    if let PyObject::Float(v) = &*obj {
        let bits = v.to_bits();
        let sign = if (bits >> 63) != 0 { "-" } else { "" };
        let biased_exp = ((bits >> 52) & 0x7ff) as i64;
        let mantissa = bits & 0x000f_ffff_ffff_ffff;
        if biased_exp == 0x7ff {
            if mantissa == 0 { Ok(py_str(&format!("{}inf", sign))) }
            else { Ok(py_str(&format!("{}nan", sign))) }
        } else if *v == 0.0 { Ok(py_str(&format!("{}0x0.0p+0", sign))) }
        else {
            let exp = biased_exp - 1023;
            let hex_mantissa = format!("{:013x}", mantissa);
            let hex_mantissa = hex_mantissa.trim_end_matches('0');
            Ok(py_str(&format!("{}0x1.{}p{:+}", sign, if hex_mantissa.is_empty() { "0" } else { hex_mantissa }, exp)))
        }
    } else { Err(PyError::type_error("hex() argument must be float")) }
}

/// Parses a real CPython-style complex literal string (`complex("1+2j")`,
/// `complex("-3-4j")`, `complex("2j")`, `complex("(1+2j)")`) — finds the
/// LAST top-level `+`/`-` before the trailing `j`/`J` (skipping one right
/// after `e`/`E`, which is an exponent sign, not the real/imag separator).
fn parse_complex_str(s: &str) -> PyResult<(f64, f64)> {
    let malformed = || PyError::value_error(format!("complex() arg is a malformed string"));
    let s = s.trim();
    let inner = s.strip_prefix('(').and_then(|s| s.strip_suffix(')')).unwrap_or(s).trim();
    if inner.is_empty() { return Err(malformed()); }
    if let Some(stripped) = inner.strip_suffix(['j', 'J']) {
        let bytes = stripped.as_bytes();
        let mut split_idx = None;
        for i in (1..bytes.len()).rev() {
            let c = bytes[i] as char;
            if c == '+' || c == '-' {
                let prev = bytes[i - 1] as char;
                if prev != 'e' && prev != 'E' {
                    split_idx = Some(i);
                    break;
                }
            }
        }
        match split_idx {
            Some(idx) => {
                let real_str = &stripped[..idx];
                let imag_str = &stripped[idx..];
                let re: f64 = real_str.parse().map_err(|_| malformed())?;
                let im: f64 = match imag_str {
                    "+" => 1.0,
                    "-" => -1.0,
                    _ => imag_str.parse().map_err(|_| malformed())?,
                };
                Ok((re, im))
            }
            None => {
                let im: f64 = match stripped {
                    "" | "+" => 1.0,
                    "-" => -1.0,
                    _ => stripped.parse().map_err(|_| malformed())?,
                };
                Ok((0.0, im))
            }
        }
    } else {
        let re: f64 = inner.parse().map_err(|_| malformed())?;
        Ok((re, 0.0))
    }
}

pub fn builtin_complex(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Ok(PyObjectRef::imm(PyObject::Complex(0.0, 0.0)));
    }
    let (re, im) = {
        let obj = args[0].borrow();
        match &*obj {
            PyObject::Complex(re, im) => (*re, *im),
            PyObject::Int(i) => (i.to_f64().unwrap_or(0.0), 0.0),
            PyObject::Float(f) => (*f, 0.0),
            PyObject::Bool(b) => (if *b { 1.0 } else { 0.0 }, 0.0),
            PyObject::Str(s) => {
                if args.len() > 1 {
                    return Err(PyError::type_error("complex() can't take second arg if first is a string"));
                }
                parse_complex_str(s)?
            }
            _ => return Err(PyError::type_error(format!("complex() argument must be a string or a number, not '{}'", obj.type_name()))),
        }
    };
    if args.len() > 1 {
        let imag_extra: f64 = {
            let obj = args[1].borrow();
            match &*obj {
                PyObject::Int(i) => i.to_f64().unwrap_or(0.0),
                PyObject::Float(f) => *f,
                PyObject::Bool(b) => if *b { 1.0 } else { 0.0 },
                _ => return Err(PyError::type_error(format!("complex() second argument must be a number, not '{}'", obj.type_name()))),
            }
        };
        return Ok(PyObjectRef::imm(PyObject::Complex(re, im + imag_extra)));
    }
    Ok(PyObjectRef::imm(PyObject::Complex(re, im)))
}

/// Does `typ`'s mro/bases include a builtin exception class (`Exception`,
/// `OSError`, ...)? Those are `PyObject::BuiltinFunction` (the exception's
/// own constructor), never a real `PyObject::Type` — invisible to
/// `lookup_dunder_via_mro`'s dict-based walk, so a plain `class MyError
/// (Exception): pass` (no custom `__str__`/`__repr__` override) fell
/// through to the fully-generic `<MyError object>` instead of real
/// `BaseException.__str__`/`__repr__`'s args-based formatting (`str(exc)`
/// for a single-arg exception should be that arg's `str()`, not a useless
/// placeholder — this broke essentially every custom exception hierarchy's
/// error messages).
/// Read a class's OWN `_abc_registry` (set by `.register()`, the generic
/// `PyObject::Type` fallback method — see its own doc comment) — NEVER via
/// `get_attribute`/`lookup_dunder_via_mro`-style MRO walking. A virtual
/// registration against one ABC (`Complex.register(complex)`) must NOT be
/// visible when checking a MORE SPECIFIC descendant ABC (`Integral`) just
/// because `Integral` inherits from `Complex` — registration doesn't flow
/// "downward" in specificity that way. (`.register()` itself has the exact
/// same "must not read via MRO" requirement, for a different reason — see
/// its own inline comment.)
fn own_abc_registry(typ: &PyObjectRef) -> Vec<PyObjectRef> {
    if let PyObject::Type { dict, .. } = &*typ.borrow() {
        dict.get_str("_abc_registry").and_then(|r| {
            if let PyObject::FrozenSet(items) = &*r.borrow() { Some(items.to_vec()) } else { None }
        }).unwrap_or_default()
    } else {
        Vec::new()
    }
}

/// Is anything matching `matcher` registered against `base` ITSELF, or
/// against any REAL (regular inheritance, non-virtual) subclass of `base`?
/// Needed because registration should propagate to less-specific ABCs in
/// the same real hierarchy: `numbers.py`'s `Integral.register(int)` must
/// also make `issubclass(int, Real)`/`issubclass(int, Complex)` true,
/// since `Integral` is a real subclass of `Real`/`Complex` (NOT because
/// `int` itself was registered against them directly — it wasn't). Walks
/// `direct_subclasses_of` recursively; cheap in practice since real ABC
/// hierarchies (`numbers`, `collections.abc`) are shallow.
fn abc_registry_matches_in_subtree(base: &PyObjectRef, matcher: &dyn Fn(&PyObjectRef) -> bool) -> bool {
    if own_abc_registry(base).iter().any(|r| matcher(r)) {
        return true;
    }
    direct_subclasses_of(base).iter().any(|sub| abc_registry_matches_in_subtree(sub, matcher))
}

fn is_exception_type(typ: &PyObjectRef) -> bool {
    if let PyObject::Type { mro, bases, .. } = &*typ.borrow() {
        let entries = if mro.is_empty() { bases } else { mro };
        entries.iter().any(|b| {
            if let PyObject::BuiltinFunction { name, .. } = &*b.borrow() {
                is_builtin_exception_class_name(name)
            } else {
                false
            }
        })
    } else {
        false
    }
}

/// Real `BaseException.__str__`: no args -> `""`, one arg -> that arg's
/// `str()`, multiple args -> `str()` of the whole args tuple.
fn exception_instance_str(instance: &PyObjectRef) -> String {
    let args = if let PyObject::Instance { dict, .. } = &*instance.borrow() {
        dict.get("args").cloned()
    } else {
        None
    };
    match args.map(|a| a.borrow().clone()) {
        Some(PyObject::Tuple(items)) if items.len() == 1 => items[0].str(),
        Some(PyObject::Tuple(items)) if !items.is_empty() => py_tuple(items).str(),
        _ => String::new(),
    }
}

/// Real `BaseException.__repr__`: `ClassName(repr(arg1), repr(arg2), ...)`.
fn exception_instance_repr(instance: &PyObjectRef, class_name: &str) -> String {
    let args = if let PyObject::Instance { dict, .. } = &*instance.borrow() {
        dict.get("args").cloned()
    } else {
        None
    };
    let args_str = match args.map(|a| a.borrow().clone()) {
        Some(PyObject::Tuple(items)) => items.iter().map(|a| a.repr()).collect::<Vec<_>>().join(", "),
        _ => String::new(),
    };
    format!("{}({})", class_name, args_str)
}

pub fn builtin_str(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() { Ok(py_str("")) }
    else {
        let f = {
            let obj_borrowed = args[0].borrow();
            if let PyObject::Instance { typ, .. } = &*obj_borrowed {
                lookup_dunder_via_mro(typ, "__str__")
            } else { None }
        };
        if let Some(f) = f {
            return call_bound_method(f, args[0].clone(), vec![]);
        }
        let is_exc = if let PyObject::Instance { typ, .. } = &*args[0].borrow() { is_exception_type(typ) } else { false };
        if is_exc {
            return Ok(py_str(&exception_instance_str(&args[0])));
        }
        Ok(py_str(&args[0].str()))
    }
}

pub fn builtin_repr(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() != 1 {
        return Err(PyError::type_error("repr() takes exactly one argument"));
    }
    let f = {
        let obj_borrowed = args[0].borrow();
        match &*obj_borrowed {
            PyObject::Instance { typ, .. } => lookup_dunder_via_mro(typ, "__repr__"),
            _ => None,
        }
    };
    if let Some(f) = f {
        return call_bound_method(f, args[0].clone(), vec![]);
    }
    let class_name = if let PyObject::Instance { typ, .. } = &*args[0].borrow() {
        if is_exception_type(typ) {
            Some(typ.borrow().type_name().to_string())
        } else {
            None
        }
    } else {
        None
    };
    if let Some(class_name) = class_name {
        return Ok(py_str(&exception_instance_repr(&args[0], &class_name)));
    }
    Ok(py_str(&args[0].repr()))
}

pub fn builtin_bool(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() > 1 { return Err(PyError::type_error("bool() takes at most 1 argument")); }
    if args.is_empty() { return Ok(py_bool(false)); }
    let typ_opt = {
        let obj = args[0].borrow();
        if let PyObject::Instance { typ, .. } = &*obj {
            if matches!(lookup_dunder_via_mro(typ, "__bool__").map(|f| f.borrow().clone()).unwrap_or(PyObject::None), PyObject::None)
                && matches!(lookup_dunder_via_mro(typ, "__len__").map(|f| f.borrow().clone()).unwrap_or(PyObject::None), PyObject::None)
            {
                None
            } else {
                Some(typ.clone())
            }
        } else {
            None
        }
    };
    if let Some(typ) = typ_opt {
        // Unlike the infallible `.truthy()` (used for implicit if/while/and/or
        // truth-testing, which must never hang even on a malformed
        // `__bool__`), the explicit `bool()` builtin CAN and must raise the
        // real CPython error when `__bool__` doesn't return an actual `bool`
        // (e.g. `def __bool__(self): return self`) — confirmed via CPython's
        // own `test_bool.test_convert_to_bool`.
        if let Some(f) = lookup_dunder_via_mro(&typ, "__bool__") {
            let result = call_bound_method(f, args[0].clone(), vec![])?;
            return match result {
                PyObjectRef::SmallBool(b) => Ok(py_bool(b)),
                other => Err(PyError::type_error(format!(
                    "__bool__ should return bool, returned {}",
                    other.borrow().type_name()
                ))),
            };
        }
        if lookup_dunder_via_mro(&typ, "__len__").is_some() {
            // Delegate to `builtin_len` itself rather than re-deriving the
            // same validation here — CPython's own `test_bool.test_sane_len`
            // asserts `bool()`'s and `len()`'s error messages for the same
            // bad `__len__` return value are byte-for-byte IDENTICAL (real
            // CPython's `bool()` calls the same `PyObject_Size` under the
            // hood); sharing this code is what guarantees that instead of
            // two hand-written messages silently drifting apart.
            let n = builtin_len(&[args[0].clone()])?;
            return Ok(py_bool(n.as_i64().unwrap_or(0) != 0));
        }
    }
    Ok(py_bool(args[0].truthy()))
}

pub fn builtin_list(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() { Ok(py_list(Vec::new())) }
    else {
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
                // Try general iteration protocol via iter() + next()
                let it = match builtin_iter(&[args[0].clone()]) {
                    Ok(it) => it,
                    Err(_) => return Err(PyError::type_error(format!("cannot convert '{}' object to list", args[0].borrow().type_name()))),
                };
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

pub fn builtin_tuple(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() { return Ok(py_tuple(Vec::new())); }
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
            let keys_method = args[0].borrow().get_attribute("keys").ok();
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
    if args.is_empty() { return Ok(py_set()); }
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
    if args.is_empty() { Ok(PyObjectRef::imm(PyObject::Bytes(Vec::new()))) }
    else {
        let obj = args[0].borrow();
        match &*obj {
            PyObject::Int(i) => {
                let n = i.to_i64().ok_or_else(|| PyError::value_error("bytes() requires int in range 0-255"))?;
                if n < 0 || n > 255 {
                    return Err(PyError::value_error("bytes() requires int in range 0-255"));
                }
                Ok(PyObjectRef::imm(PyObject::Bytes(vec![n as u8])))
            }
            PyObject::Bytes(b) => Ok(PyObjectRef::imm(PyObject::Bytes(b.clone()))),
            PyObject::ByteArray(b) => Ok(PyObjectRef::imm(PyObject::Bytes(b.clone()))),
            PyObject::Str(s) => Ok(PyObjectRef::imm(PyObject::Bytes(s.as_bytes().to_vec()))),
            PyObject::List(v) => {
                let mut result = Vec::new();
                for item in v {
                    let item = item.borrow();
                    if let PyObject::Int(i) = &*item {
                        let n = i.to_i64().ok_or_else(|| PyError::value_error("bytes() requires int in range 0-255"))?;
                        if n < 0 || n > 255 {
                            return Err(PyError::value_error("bytes() requires int in range 0-255"));
                        }
                        result.push(n as u8);
                    } else {
                        return Err(PyError::type_error("bytes() argument must be an integer or iterable"));
                    }
                }
                Ok(PyObjectRef::imm(PyObject::Bytes(result)))
            }
            PyObject::Tuple(v) => {
                let mut result = Vec::new();
                for item in v {
                    let item = item.borrow();
                    if let PyObject::Int(i) = &*item {
                        let n = i.to_i64().ok_or_else(|| PyError::value_error("bytes() requires int in range 0-255"))?;
                        if n < 0 || n > 255 {
                            return Err(PyError::value_error("bytes() requires int in range 0-255"));
                        }
                        result.push(n as u8);
                    } else {
                        return Err(PyError::type_error("bytes() argument must be an integer or iterable"));
                    }
                }
                Ok(PyObjectRef::imm(PyObject::Bytes(result)))
            }
            PyObject::Set(items) | PyObject::FrozenSet(items) => {
                let mut result = Vec::new();
                for item in items.to_vec() {
                    let item_b = item.borrow();
                    if let PyObject::Int(i) = &*item_b {
                        let n = i.to_i64().ok_or_else(|| PyError::value_error("bytes() requires int in range 0-255"))?;
                        if n < 0 || n > 255 {
                            return Err(PyError::value_error("bytes() requires int in range 0-255"));
                        }
                        result.push(n as u8);
                    } else {
                        return Err(PyError::type_error("bytes() argument must be integer or iterable"));
                    }
                }
                Ok(PyObjectRef::imm(PyObject::Bytes(result)))
            }
            _ => {
                drop(obj);
                let it = match builtin_iter(&[args[0].clone()]) {
                    Ok(it) => it,
                    Err(_) => return Err(PyError::type_error(format!("cannot convert '{}' to bytes", args[0].borrow().type_name()))),
                };
                let mut result = Vec::new();
                loop {
                    let item = match builtin_next(&[it.clone()]) {
                        Ok(val) => val,
                        Err(PyError::StopIteration) => break,
                        Err(e) => return Err(e),
                    };
                    let item_b = item.borrow();
                    if let PyObject::Int(i) = &*item_b {
                        let n = i.to_i64().ok_or_else(|| PyError::value_error("bytes() requires int in range 0-255"))?;
                        if n < 0 || n > 255 {
                            return Err(PyError::value_error("bytes() requires int in range 0-255"));
                        }
                        result.push(n as u8);
                    } else {
                        return Err(PyError::type_error("bytes() argument must be an integer or iterable"));
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
        return Err(PyError::type_error("bytes.fromhex() takes exactly 1 argument (0 given)"));
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
        let hex_pair = std::str::from_utf8(chunk).map_err(|_| {
            PyError::value_error("non-hexadecimal number found")
        })?;
        let byte = u8::from_str_radix(hex_pair, 16).map_err(|_| {
            PyError::value_error(format!("non-hexadecimal number found in fromhex() arg at position {}", s.find(hex_pair).unwrap_or(0)))
        })?;
        result.push(byte);
    }
    Ok(PyObjectRef::imm(PyObject::Bytes(result)))
}

pub fn builtin_bytearray(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() { Ok(PyObjectRef::new(PyObject::ByteArray(Vec::new()))) }
    else {
        let obj = args[0].borrow();
        match &*obj {
            PyObject::Int(i) => {
                let n = i.to_i64().ok_or_else(|| PyError::value_error("bytearray() requires int in range 0-255"))?;
                if n < 0 || n > 255 {
                    return Err(PyError::value_error("bytearray() requires int in range 0-255"));
                }
                Ok(PyObjectRef::new(PyObject::ByteArray(vec![n as u8])))
            }
            PyObject::Bytes(b) => Ok(PyObjectRef::new(PyObject::ByteArray(b.clone()))),
            PyObject::ByteArray(b) => Ok(PyObjectRef::new(PyObject::ByteArray(b.clone()))),
            PyObject::Str(s) => Ok(PyObjectRef::new(PyObject::ByteArray(s.as_bytes().to_vec()))),
            PyObject::List(v) => {
                let mut result = Vec::new();
                for item in v {
                    let item = item.borrow();
                    if let PyObject::Int(i) = &*item {
                        let n = i.to_i64().ok_or_else(|| PyError::value_error("bytearray() requires int in range 0-255"))?;
                        if n < 0 || n > 255 {
                            return Err(PyError::value_error("bytearray() requires int in range 0-255"));
                        }
                        result.push(n as u8);
                    } else {
                        return Err(PyError::type_error("bytearray() argument must be an integer or iterable"));
                    }
                }
                Ok(PyObjectRef::new(PyObject::ByteArray(result)))
            }
            PyObject::Tuple(v) => {
                let mut result = Vec::new();
                for item in v {
                    let item = item.borrow();
                    if let PyObject::Int(i) = &*item {
                        let n = i.to_i64().ok_or_else(|| PyError::value_error("bytearray() requires int in range 0-255"))?;
                        if n < 0 || n > 255 {
                            return Err(PyError::value_error("bytearray() requires int in range 0-255"));
                        }
                        result.push(n as u8);
                    } else {
                        return Err(PyError::type_error("bytearray() argument must be an integer or iterable"));
                    }
                }
                Ok(PyObjectRef::new(PyObject::ByteArray(result)))
            }
            _ => Err(PyError::type_error(format!("cannot convert '{}' to bytearray", obj.type_name()))),
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
                for item in v { set.add(item.clone())?; }
                Ok(PyObjectRef::imm(PyObject::FrozenSet(set)))
            }
            PyObject::Tuple(v) => {
                let mut set = PySet::new();
                for item in v { set.add(item.clone())?; }
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
            _ => Err(PyError::type_error(format!("cannot convert '{}' to frozenset", obj.type_name()))),
        }
    }
}

pub fn builtin_format(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    match args.len() {
        0 => Err(PyError::type_error("format() requires at least 1 argument")),
        1 => Ok(py_str(&args[0].str())),
        2 => {
            let spec = args[1].str();
            if spec.trim().is_empty() {
                return Ok(py_str(&args[0].str()));
            }
            // Use the comprehensive format_with_spec from vm.rs
            let result = crate::vm::format_with_spec(&args[0], &spec)
                .map_err(|e| PyError::value_error(format!("Format spec: {}", e)))?;
            Ok(py_str(&result))
        }
        _ => Err(PyError::type_error("format() takes at most 2 arguments")),
    }
}

pub fn builtin_object(_args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    // Create a new bare object instance
    let object_type = PyObjectRef::new(PyObject::Type {
        name: "object".to_string(),
        dict: Box::new(TypeDict::default()),
        bases: vec![],
        mro: vec![],
    });
    Ok(PyObjectRef::new(PyObject::Instance {
        typ: object_type,
        dict: AttrMap::new(),
    }))
}

pub fn builtin_hash(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() != 1 {
        return Err(PyError::type_error("hash() takes exactly one argument"));
    }
    // Delegate entirely to `PyObjectRef::hash()` — the SAME method
    // `PyDict`/`PySet` call internally to hash a key. This used to
    // reimplement a SEPARATE algorithm per type here (FNV-1a for
    // str/bytes/bytearray/tuple, vs. the byte-multiplier/char-multiplier
    // algorithms `PyObjectRef::hash()`/`PyObject::hash()` actually use for
    // dict/set storage) — meaning `hash(x)` as seen by Python code could
    // disagree with the hash ACTUALLY used when `x` is placed in a dict/set,
    // a correctness invariant `hash()` exists specifically to uphold.
    // Confirmed via `hash("hello")` returning a completely different value
    // depending on whether "hello" happened to be represented as an inline
    // `SmallStr` or a boxed `PyObject::Str` — both must, and now do, agree.
    // This also fixes a second bug: the old `_` catch-all silently hashed
    // genuinely unhashable types (`list`, `dict`, `set`) by pointer instead
    // of raising `TypeError: unhashable type: '...'` like real Python.
    Ok(py_int(args[0].hash()? as i64))
}

pub fn builtin_slice(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    match args.len() {
        1 => {
            let stop = args[0].clone();
            let none = py_none();
            Ok(PyObjectRef::imm(PyObject::Slice {
                start: none.clone(),
                stop,
                step: none,
            }))
        }
        2 => {
            let start = args[0].clone();
            let stop = args[1].clone();
            let none = py_none();
            Ok(PyObjectRef::imm(PyObject::Slice {
                start,
                stop,
                step: none,
            }))
        }
        3 => {
            Ok(PyObjectRef::imm(PyObject::Slice {
                start: args[0].clone(),
                stop: args[1].clone(),
                step: args[2].clone(),
            }))
        }
        _ => Err(PyError::type_error("slice() takes at most 3 arguments")),
    }
}

pub fn builtin_dir(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Ok(py_list(Vec::new()));
    }
    let obj = args[0].borrow();
    let mut names = Vec::new();
    match &*obj {
        PyObject::Instance { dict, .. } => {
            for key in dict.keys() {
                names.push(py_str(key));
            }
        }
        PyObject::Module { dict, .. } => {
            for key in dict.keys() {
                names.push(py_str(interner::lookup_str(*key)));
            }
        }
        PyObject::Type { dict, mro, .. } => {
            // `dir(SomeClass)` must include every name reachable via the
            // class's OWN dict AND every ancestor's (`mro`) — real
            // CPython's `dir()` on a class is `sorted(set().union(*(vars(c)
            // for c in cls.__mro__)))`. This previously only read the
            // class's OWN dict, so `dir()` on ANY class with inherited
            // members (i.e. virtually every class with a base other than
            // bare `object`) silently omitted every name defined on a
            // parent — confirmed via `class Combined(Mixin, unittest.
            // TestCase): pass`, where `dir(Combined)` omitted `Mixin`'s own
            // `test_*` methods even though `hasattr`/`getattr` found them
            // fine (a real, general attribute-LOOKUP-vs-`dir()`-enumeration
            // gap this was hiding) — `unittest`'s own `TestLoader.
            // getTestCaseNames` uses exactly this `dir()` call to discover
            // test methods, so this silently dropped every test defined on
            // a mixin base for any multiple-inheritance test class.
            let mut seen = std::collections::HashSet::new();
            for key in dict.keys() {
                if seen.insert(*key) { names.push(py_str(interner::lookup_str(*key))); }
            }
            for base in mro {
                if let PyObject::Type { dict: base_dict, .. } = &*base.borrow() {
                    for key in base_dict.keys() {
                        if seen.insert(*key) { names.push(py_str(interner::lookup_str(*key))); }
                    }
                }
            }
        }
        _ => {}
    }
    // Add basic attributes for all types
    names.push(py_str("__class__"));
    names.push(py_str("__dir__"));
    names.sort_by(|a, b| {
        let a = a.borrow();
        let b = b.borrow();
        if let (PyObject::Str(a), PyObject::Str(b)) = (&*a, &*b) {
            a.cmp(b)
        } else { std::cmp::Ordering::Equal }
    });
    Ok(py_list(names))
}

pub fn builtin_globals(_args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    with_vm_mut(|vm| {
        let frame = vm.frames.last().ok_or_else(|| PyError::runtime_error("no frame"))?;
        let globals = frame.globals.borrow();
        let mut d = crate::object::PyDict::new();
        for (k, v) in globals.iter() {
            d.set(py_str(interner::lookup_str(*k)), v.clone())?;
        }
        Ok(PyObjectRef::new(PyObject::Dict(Box::new(d))))
    })?
}

pub fn builtin_locals(_args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    with_vm_mut(|vm| {
        let frame = vm.frames.last().ok_or_else(|| PyError::runtime_error("no frame"))?;
        let mut d = crate::object::PyDict::new();
        for (k, v) in frame.locals.iter() {
            let name = crate::interner::lookup(k);
            d.set(py_str(&name), v.clone())?;
        }
        Ok(PyObjectRef::new(PyObject::Dict(Box::new(d))))
    })?
}

pub fn builtin_divmod(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() != 2 { return Err(PyError::type_error("divmod() takes exactly 2 arguments")); }
    let a = args[0].as_i64().ok_or_else(|| PyError::type_error("divmod() arg must be int"))?;
    let b = args[1].as_i64().ok_or_else(|| PyError::type_error("divmod() arg must be int"))?;
    if b == 0 { return Err(PyError::value_error("division by zero")); }
    Ok(PyObjectRef::new(PyObject::Tuple(vec![py_int(a / b), py_int(a % b)])))
}

pub fn builtin_round(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 1 || args.len() > 2 { return Err(PyError::type_error("round() takes 1 or 2 arguments")); }
    let val = {
        let obj = args[0].borrow();
        match &*obj {
            PyObject::Int(i) => i.to_f64().unwrap_or(0.0),
            PyObject::Float(f) => *f,
            _ => return Err(PyError::type_error("round() arg must be numeric")),
        }
    };
    if args.len() == 2 {
        let n = args[1].as_i64().ok_or_else(|| PyError::type_error("ndigits must be int"))? as i32;
        Ok(py_float((val * 10_f64.powi(n)).round() / 10_f64.powi(n)))
    } else {
        Ok(py_int(val.round() as i64))
    }
}

pub fn builtin_abs(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() != 1 {
        return Err(PyError::type_error("abs() takes exactly one argument"));
    }
    let obj = args[0].borrow();
    match &*obj {
        PyObject::Int(i) => Ok(py_int(i.clone().abs())),
        PyObject::Float(f) => Ok(py_float(f.abs())),
        PyObject::Complex(re, im) => Ok(py_float(re.hypot(*im))),
        PyObject::Instance { typ, .. } => {
            match lookup_dunder_via_mro(typ, "__abs__") {
                Some(f) => {
                    drop(obj);
                    call_bound_method(f, args[0].clone(), vec![])
                }
                None => Err(PyError::type_error(format!("bad operand type for abs(): '{}'", get_type_name_for_instance(typ)))),
            }
        }
        _ => Err(PyError::type_error(format!("bad operand type for abs(): '{}'", obj.type_name()))),
    }
}

pub fn builtin_hasattr(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() != 2 {
        return Err(PyError::type_error("hasattr() takes exactly 2 arguments"));
    }
    let attr_name = args[1].str();
    if std::env::var("RPY_DEBUG_GETATTR").is_ok() {
        eprintln!("HASATTR: obj_type={} attr={}", args[0].borrow().type_name(), attr_name);
    }
    match args[0].borrow().get_attribute(&attr_name) {
        Ok(_) => Ok(py_bool(true)),
        Err(_) => Ok(py_bool(false)),
    }
}

pub fn builtin_getattr(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 2 {
        return Err(PyError::type_error("getattr() takes at least 2 arguments"));
    }
    let attr_name = args[1].str();
    match args[0].borrow().get_attribute(&attr_name) {
        Ok(val) => {
            // `get_attribute`'s own `PyObject::Type` handling unwraps
            // `StaticMethod` descriptors but NOT `ClassMethod` ones — that
            // binding is instead done separately, only inside vm.rs's
            // `LOAD_ATTR` opcode handler (which has direct access to a
            // `PyObjectRef` for the class to bind against; `get_attribute`
            // only gets `&self`/`&PyObject`, with no such handle). That
            // meant `Foo.bar()` (going through `LOAD_ATTR`) correctly
            // called a `@classmethod`-decorated `bar`, but
            // `getattr(Foo, 'bar')()` returned the raw, uncallable
            // `ClassMethod` descriptor object instead — `TypeError:
            // 'classmethod' object is not callable`. Real trigger:
            // `unittest.suite.py`'s `getattr(currentClass, 'setUpClass',
            // None)` — every single `TestCase` subclass's default
            // `@classmethod setUpClass`/`tearDownClass` hit this.
            if matches!(&*args[0].borrow(), PyObject::Type { .. }) {
                if let PyObject::ClassMethod { func } = &*val.borrow() {
                    return Ok(PyObjectRef::new(PyObject::BoundMethod {
                        func: func.clone(),
                        self_obj: args[0].clone(),
                    }));
                }
            }
            Ok(val)
        }
        Err(_) if args.len() >= 3 => Ok(args[2].clone()),
        Err(e) => Err(e),
    }
}

pub fn builtin_setattr(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() != 3 {
        return Err(PyError::type_error("setattr() takes exactly 3 arguments"));
    }
    let attr_name = args[1].str();
    // `.borrow_mut()` panics unconditionally for anything that ISN'T
    // `PyObjectRef::Mut` — that's every inline variant (SmallInt/SmallBool/
    // SmallFloat/SmallStr/None, no backing RefCell at all) AND every
    // `Imm`-wrapped value (boxed Int/Str/Float, Tuple, Bytes, Function,
    // Code, Type — immutable by this codebase's design, even though real
    // CPython DOES allow setting arbitrary attributes on a plain function).
    // A previous fix here only covered the inline variants, so
    // `setattr(some_function, 'x', 1)` (a real CPython feature we don't
    // support, but a common thing for tests to exercise, e.g. CPython's own
    // `test_funcattrs.py`) still crashed the whole process. Raising the
    // same `AttributeError` real CPython gives for a genuinely
    // attribute-less type is a strictly better fallback than a crash, even
    // where CPython itself would have allowed it.
    if !matches!(args[0], PyObjectRef::Mut(_)) {
        return Err(PyError::attribute_error(format!(
            "'{}' object has no attribute '{}'", args[0].borrow().type_name(), attr_name
        )));
    }
    args[0].borrow_mut().set_attribute(&attr_name, args[2].clone())?;
    Ok(py_none())
}

pub fn builtin_delattr(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() != 2 {
        return Err(PyError::type_error("delattr() takes exactly 2 arguments"));
    }
    let attr_name = args[1].str();
    // Check for __delattr__ on Instance types first
    let f = {
        let obj_borrowed = args[0].borrow();
        match &*obj_borrowed {
            PyObject::Instance { typ, .. } => lookup_dunder_via_mro(typ, "__delattr__"),
            _ => None,
        }
    };
    if let Some(f) = f {
        return call_bound_method(f, args[0].clone(), vec![args[1].clone()]);
    }
    // See the matching guard in `builtin_setattr` just above.
    if !matches!(args[0], PyObjectRef::Mut(_)) {
        return Err(PyError::attribute_error(format!(
            "'{}' object has no attribute '{}'", args[0].borrow().type_name(), attr_name
        )));
    }
    args[0].borrow_mut().del_attribute(&attr_name)?;
    Ok(py_none())
}

pub fn builtin_ord(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() != 1 {
        return Err(PyError::type_error("ord() takes exactly one argument"));
    }
    let s = args[0].str();
    let c = s.chars().next().ok_or_else(|| PyError::type_error("ord() expected a character, but string of length 0 found"))?;
    Ok(py_int(c as u32 as i64))
}

pub fn builtin_chr(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() != 1 {
        return Err(PyError::type_error("chr() takes exactly one argument"));
    }
    let n = to_index(&args[0])?;
    let code = n.to_usize().ok_or_else(|| PyError::value_error("chr() arg not in range(0x110000)"))?;
    let c = char::from_u32(code as u32).ok_or_else(|| PyError::value_error("chr() arg not in range(0x110000)"))?;
    Ok(py_str(&c.to_string()))
}

pub fn builtin_hex(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() != 1 {
        return Err(PyError::type_error("hex() takes exactly one argument"));
    }
    let n = to_index(&args[0])?;
    Ok(py_str(&format!("0x{:x}", n)))
}

pub fn builtin_oct(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() != 1 {
        return Err(PyError::type_error("oct() takes exactly one argument"));
    }
    let n = to_index(&args[0])?;
    Ok(py_str(&format!("0o{:o}", n)))
}

pub fn builtin_bin(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() != 1 {
        return Err(PyError::type_error("bin() takes exactly one argument"));
    }
    let n = to_index(&args[0])?;
    Ok(py_str(&format!("0b{:b}", n)))
}

pub fn builtin_ascii(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() != 1 {
        return Err(PyError::type_error("ascii() takes exactly one argument"));
    }
    let s = args[0].repr();
    let mut result = String::with_capacity(s.len());
    for c in s.chars() {
        if c.is_ascii() {
            result.push(c);
        } else {
            let code = c as u32;
            if code <= 0xFF {
                result.push_str(&format!("\\x{:02x}", code));
            } else if code <= 0xFFFF {
                result.push_str(&format!("\\u{:04x}", code));
            } else {
                result.push_str(&format!("\\U{:08x}", code));
            }
        }
    }
    Ok(py_str(&result))
}

pub fn builtin_memoryview(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() != 1 {
        return Err(PyError::type_error("memoryview() takes exactly one argument"));
    }
    let obj = args[0].borrow();
    match &*obj {
        PyObject::Bytes(b) => Ok(PyObjectRef::new(PyObject::ByteArray(b.clone()))),
        PyObject::ByteArray(b) => Ok(PyObjectRef::new(PyObject::ByteArray(b.clone()))),
        _ => Err(PyError::type_error("memoryview: unsupported type")),
    }
}

pub fn builtin_input(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if !args.is_empty() {
        print!("{}", args[0].str());
    }
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).map_err(|e| PyError::runtime_error(e.to_string()))?;
    if line.ends_with('\n') {
        line.pop();
    }
    Ok(py_str(&line))
}

pub fn builtin_exit(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let code = if args.is_empty() { 0 }
    else if let PyObject::Int(i) = &*args[0].borrow() {
        i.to_i32().unwrap_or(0)
    } else { 0 };
    Err(PyError::SystemExit(code))
}

pub fn call_bound_method(func: PyObjectRef, self_obj: PyObjectRef, args: Vec<PyObjectRef>) -> PyResult<PyObjectRef> {
    match &*func.borrow() {
        PyObject::BuiltinMethod { func: f, self_obj: s, .. } => {
            let mut all_args = vec![s.clone()];
            all_args.push(self_obj);
            all_args.extend(args);
            f(&all_args)
        }
        PyObject::BuiltinFunction { func: f, .. } => {
            let mut all_args = vec![self_obj];
            all_args.extend(args);
            f(&all_args)
        }
        PyObject::Closure(func) => {
            let mut all_args = vec![self_obj];
            all_args.extend(args);
            func(&all_args)
        }
        PyObject::Function(ref f) => {
            let code = &f.code;
            let g = &f.globals;
            let defaults = &f.defaults;
            let fname = &f.code.name;
            let closure = &f.closure;
            if std::env::var("RPY_DEBUG_IMPORT").is_ok() {
                eprintln!("CALL_BOUND_METHOD (disposable VM): fname={} code_name={} filename={}", fname, code.name, code.filename);
            }
            if std::env::var("RPY_DEBUG_CBM").is_ok() {
                eprintln!("call_bound_method: fname={} varnames={:?} args.len()={} arg_count={}", fname, code.varnames, args.len(), code.arg_count);
            }
            // The disposable VM is constructed BEFORE the frame (and the
            // frame borrows ITS `builtins` map, not a separately-built one)
            // deliberately: `vm.rs`'s `call_function` special-cases `type(x)`
            // (and a few other builtins) by POINTER IDENTITY against
            // `self.builtins.get("type")` — if the frame's own `builtins` map
            // were built via a second, independent `create_builtins()` call
            // (as this used to do), it would contain a structurally-identical
            // but NOT pointer-identical "type" object, so that identity check
            // silently fails and `type(x)` falls through to being treated as
            // an ordinary call to the `type` class (constructing a bogus
            // instance-of-`type`) instead of returning `x`'s real type.
            // Confirmed general via a Django-free repro: `type(self).__name__`
            // inside a function invoked through `call_bound_method` (e.g. any
            // user-defined `__repr__` reached via the `repr()` builtin)
            // raised `AttributeError: 'type' object has no attribute
            // '__name__'` — `type(self)` had silently returned a fresh
            // `type`-instance instead of the real class object.
            let mut vm = crate::vm::VirtualMachine::new();
            let mut frame = crate::vm::Frame::new(code.clone(), g.clone(), std::rc::Rc::clone(&vm.builtins), None);
            // Without this, ANY closure-capturing function invoked via
            // call_bound_method (repr()/str()/hash()/comparisons/other
            // native builtins that call a user-defined dunder this way,
            // instead of through the normal CALL opcode's own frame setup
            // in vm.rs, which already does set this) silently lost every
            // free variable — "variable 'x' not found" the moment the
            // function's body referenced one. Confirmed general via a
            // Django-free repro: a `dataclass`-generated `__repr__` closing
            // over its own field-name list worked fine called directly
            // (`obj.__repr__()`) but raised NameError through `repr(obj)`.
            frame.closure = Box::new(closure.clone());
            let code = code.clone();
            let defaults = defaults.clone();
            // Set self at index 0
            if !code.varnames.is_empty() {
                frame.fast_locals[0] = Some(self_obj.clone());
                frame.insert_local(crate::interner::lookup_str(code.varnames[0]), self_obj);
            }
            let npos = args.len();
            let named_params = if code.vararg_name.is_some() || code.kwarg_name.is_some() {
                code.varnames.iter().position(|n| {
                    code.vararg_name.as_ref().map(|b| b.as_str()) == Some(crate::interner::lookup_str(*n)) || code.kwarg_name.as_ref().map(|b| b.as_str()) == Some(crate::interner::lookup_str(*n))
                }).unwrap_or(code.varnames.len())
            } else {
                code.varnames.len()
            };
            for i in 0..npos.min(named_params.saturating_sub(1)) {
                let idx = i + 1;
                if idx < code.varnames.len() {
                    frame.fast_locals[idx] = Some(args[i].clone());
                    frame.insert_local(crate::interner::lookup_str(code.varnames[idx]), args[i].clone());
                }
            }
            if let Some(vararg_name) = &code.vararg_name {
                let mut extra = Vec::new();
                for i in (named_params.saturating_sub(1))..npos {
                    extra.push(args[i].clone());
                }
                frame.insert_local(vararg_name.as_str(), py_tuple(extra));
            }
            if npos < named_params.saturating_sub(1) {
                let num_defaults = code.num_defaults;
                for i in npos..named_params.saturating_sub(1) {
                    let idx = i + 1;
                    if idx < code.varnames.len() {
                        let default_idx = num_defaults.saturating_sub(named_params.saturating_sub(1) - i);
                        if default_idx < defaults.len() {
                            let val = defaults[default_idx].clone();
                            frame.fast_locals[idx] = Some(val.clone());
                            frame.insert_local(crate::interner::lookup_str(code.varnames[idx]), val);
                        }
                    }
                }
            }
            if std::env::var("RPY_DEBUG_CBM").is_ok() {
                eprintln!("call_bound_method: fast_locals after setup = {:?}", frame.fast_locals.iter().map(|v| v.as_ref().map(|x| x.repr())).collect::<Vec<_>>());
            }
            vm.frames.push(frame);
            vm.execute()
        }
        PyObject::BoundMethod { func, .. } => {
            let mut all_args = vec![self_obj.clone()];
            all_args.extend(args);
            call_bound_method(func.clone(), self_obj, all_args)
        }
        // The single-argument form of the `type` builtin itself (`type(x)`
        // — get x's real type) reaching this path as a plain callable
        // value, e.g. passed as a `key=` function to a native higher-order
        // builtin (`itertools.groupby(data, type)` — real trigger:
        // CPython's own `Lib/statistics.py`). `type` is represented as a
        // real `PyObject::Type{name:"type",..}` here (not a
        // `BuiltinFunction`, unlike most other native callables), so it
        // fell through to the generic "object is not callable" error
        // otherwise. Delegates to the same `builtin_type_of` helper the
        // real CALL-opcode path and `.__class__` both already use — not a
        // new implementation. Other `PyObject::Type` calls (constructing an
        // actual instance, or `type(name, bases, dict)`) still aren't
        // supported through THIS path — only real Python code going
        // through the live VM's own `call_function` gets that; this is
        // scoped to the one case that's actually reachable from a native
        // higher-order builtin's disposable-VM-free call convention.
        PyObject::Type { name, .. } if name == "type" && args.is_empty() => {
            builtin_type_of(&[self_obj])
        }
        _ => Err(PyError::type_error("object is not callable")),
    }
}

pub fn builtin_sorted(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error("sorted() takes at least 1 argument"));
    }
    // Check for key keyword argument (last arg could be a dict with "key")
    let key_fn: Option<PyObjectRef> = if args.len() >= 2 {
        // Check if last arg is a dict (keyword args container)
        let last = args.last().unwrap();
        let last_borrowed = last.borrow();
        if let PyObject::Dict(kwargs) = &*last_borrowed {
            kwargs.get(&py_str("key")).unwrap_or(None)
        } else {
            None
        }
    } else {
        None
    };
    let mut v = Vec::new();
    let iterable = builtin_iter(&[args[0].clone()])?;
    loop {
        match builtin_next(&[iterable.clone()]) {
            Ok(val) => v.push(val),
            Err(PyError::StopIteration) => break,
            Err(e) => return Err(e),
        }
    }
    // Sort with comparison, optionally applying key function. Uses the
    // panic-tolerant `py_stable_sort_by` (see its own doc comment) rather
    // than `Vec::sort_by`, since a deliberately-inconsistent comparator
    // (real CPython test: `test_sort.py`'s `test_bug453523`) makes the
    // standard library's sort abort the whole process.
    let len = v.len();
    if len > 1 {
        let key_fn_ref = key_fn.clone();
        v = py_stable_sort_by(v, &|a, b| {
            let a_val = if let Some(ref kf) = key_fn_ref {
                call_bound_method(kf.clone(), a.clone(), vec![]).unwrap_or_else(|_| a.clone())
            } else {
                a.clone()
            };
            let b_val = if let Some(ref kf) = key_fn_ref {
                call_bound_method(kf.clone(), b.clone(), vec![]).unwrap_or_else(|_| b.clone())
            } else {
                b.clone()
            };
            // Route through py_compare (not the raw Compare trait methods)
            // so user-defined classes' __lt__/__gt__ are consulted — the
            // trait impl alone has no notion of Instance dunder dispatch.
            py_compare(&a_val, &b_val, 0).map(|r| r.truthy()).unwrap_or(false)
        });
    }
    Ok(py_list(v))
}

pub fn builtin_enumerate(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error("enumerate() takes at least 1 argument"));
    }
    let start: usize = if args.len() > 1 {
        if let PyObject::Int(i) = &*args[1].borrow() {
            i.to_usize().unwrap_or(0)
        } else { 0 }
    } else { 0 };
    // Lazily wrap the source iterator — see `PyObject::EnumerateIter`'s own
    // doc comment for why eagerly draining it here (the previous approach)
    // was a real bug, not just a style choice.
    let iterable = builtin_iter(&[args[0].clone()])?;
    Ok(PyObjectRef::new(PyObject::EnumerateIter { source: iterable, pos: 0, start }))
}

pub fn builtin_iter(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() != 1 {
        return Err(PyError::type_error("iter() takes exactly one argument"));
    }
    // Check for __iter__ on instances
    let f = {
        let obj = args[0].borrow();
        match &*obj {
            PyObject::Instance { typ, .. } => lookup_dunder_via_mro(typ, "__iter__"),
            PyObject::Generator { .. } => {
                // Generators are their own iterator (return self)
                return Ok(args[0].clone());
            }
            // A class object itself, iterable via its metaclass's
            // `__iter__` (e.g. `iter(SomeEnum)` / `list(SomeEnum)`) — see
            // the matching GET_ITER opcode handling in vm.rs for why this
            // needs metatype_of rather than ordinary attribute lookup.
            PyObject::Type { .. } => metatype_of(&args[0]).and_then(|mt| lookup_dunder_via_mro(&mt, "__iter__")),
            _ => None,
        }
    };
    if let Some(f) = f {
        return call_bound_method(f, args[0].clone(), vec![]);
    }
    if let Some(native) = native_backing_of(&args[0]) {
        return builtin_iter(&[native]);
    }
    let obj = args[0].borrow();
    match &*obj {
        PyObject::Tuple(v) => Ok(py_list(v.clone())),
        PyObject::Str(s) => Ok(py_list(s.chars().map(|c| py_str(&c.to_string())).collect())),
        PyObject::Bytes(b) => Ok(PyObjectRef::new(PyObject::ListIter { list: b.iter().map(|byte| py_int(*byte as i64)).collect(), index: 0 })),
        PyObject::ByteArray(b) => Ok(PyObjectRef::new(PyObject::ListIter { list: b.iter().map(|byte| py_int(*byte as i64)).collect(), index: 0 })),
        // `iter(a_set)` must return a real ITERATOR (advanceable via
        // `builtin_next`), not the bare materialized list `py_list` builds —
        // a raw `PyObject::List` isn't itself a valid iterator shape (unlike
        // `List`/`Dict` just above and below, both correctly wrapped in
        // `ListIter`). This meant `for x in frozenset(...)`/`for x in
        // some_set:` raised `TypeError: 'frozenset' object is not
        // iterable` outright — a foundational gap for two of Python's most
        // basic builtin container types. Real trigger: vendoring
        // `_strptime.py` (`for i in calendar.day_abbr` style iteration
        // deeper in its own dependency chain hits a frozenset somewhere in
        // `unicodedata`/locale data) — but the bug is general, not specific
        // to that file.
        PyObject::Set(s) => Ok(PyObjectRef::new(PyObject::ListIter { list: s.to_vec(), index: 0 })),
        PyObject::FrozenSet(s) => Ok(PyObjectRef::new(PyObject::ListIter { list: s.to_vec(), index: 0 })),
        PyObject::Range { start, stop, step } => {
            Ok(PyObjectRef::new(PyObject::RangeIter { current: *start, stop: *stop, step: *step }))
        }
        PyObject::List(v) => {
            Ok(PyObjectRef::new(PyObject::ListIter { list: v.clone(), index: 0 }))
        }
        PyObject::Dict(d) => {
            Ok(PyObjectRef::new(PyObject::ListIter { list: d.keys(), index: 0 }))
        }
        // Already an iterator object (one of `builtin_next`'s own
        // recognized variants) — `iter(it)` on an existing iterator
        // just returns it unchanged, matching real Python.
        PyObject::ListIter { .. } | PyObject::RangeIter { .. } | PyObject::CycleIter { .. }
        | PyObject::EnumerateIter { .. } | PyObject::MapIterator { .. } | PyObject::FilterIterator { .. }
        | PyObject::ZipIterator { .. } | PyObject::FutureAwaitIterator { .. } | PyObject::GroupByIter { .. } => Ok(args[0].clone()),
        // Anything else (plain functions, ints, ...) is genuinely not
        // iterable. The previous fallback (`Ok(args[0].clone())`)
        // silently treated ANY object as if it were already a valid
        // iterator instead of raising here — `builtin_next` then had no
        // recognized shape to advance either, and its OWN fallback
        // apparently tried calling the object as if `__next__` meant
        // "call it", reentrantly re-borrowing the same `RefCell` and
        // panicking with "RefCell already borrowed" instead of a clean
        // `TypeError` (confirmed via `operator.countOf(countOf, countOf)`
        // — a non-iterable `BuiltinFunction` passed to `iter()` — from
        // CPython's own `test_iter.py::test_countOf`).
        other => Err(PyError::type_error(format!("'{}' object is not iterable", other.type_name()))),
    }
}

pub fn builtin_next(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 1 {
        return Err(PyError::type_error("next() takes at least 1 argument"));
    }
    // Check for __next__ on instances
    let f = {
        let obj = args[0].borrow();
        match &*obj {
            PyObject::Instance { typ, .. } => lookup_dunder_via_mro(typ, "__next__"),
            PyObject::Generator { .. } => {
                drop(obj);
                let next_func = args[0].borrow().get_attribute("__next__")?;
                let (_n, f) = {
                    let b = next_func.borrow();
                    if let PyObject::BuiltinMethod { name, func, .. } = &*b {
                        (name.clone(), *func)
                    } else { return Err(PyError::runtime_error("expected __next__ method")) }
                };
                let result = f(&[args[0].clone()]);
                // Convert raise StopIteration into PyError::StopIteration for next() protocol
                if let Err(ref e) = result {
                    if is_stop_iteration_error(e) {
                        return Err(PyError::StopIteration);
                    }
                }
                return result;
            }
            _ => None,
        }
    };
    if let Some(f) = f {
        let result = call_bound_method(f, args[0].clone(), vec![]);
        // Convert raise StopIteration into PyError::StopIteration for next() protocol
        if let Err(PyError::Exception(_, ref exc)) = result {
            let is_stop = match &*exc.borrow() {
                PyObject::Exception { typ, .. } if typ == "StopIteration" => true,
                _ => false,
            };
            if is_stop {
                return Err(PyError::StopIteration);
            }
        }
        return result;
    }
    // Fallback to list-based iteration
    // Inline types (SmallInt etc.) are not iterable — return TypeError
    // without calling borrow_mut on something that doesn't support it.
    match args[0] {
        PyObjectRef::SmallInt(_) | PyObjectRef::SmallBool(_) | PyObjectRef::SmallFloat(_) | PyObjectRef::SmallStr(_) | PyObjectRef::None => {
            return Err(PyError::type_error(format!("'{}' object is not an iterator", args[0].get_type_name())));
        }
        _ => {}
    }
    // `GroupByIter` handled as its own, separate pre-check (not inside the
    // `match &mut *obj` below) because its advance logic must call
    // arbitrary Python code (the key function, `equals()` on keys) WITHOUT
    // holding this object's own `borrow_mut()` — otherwise a reentrant
    // `next()` on this SAME groupby object from within that callback
    // (real, deliberately adversarial CPython regression test:
    // `test_groupby_reentrant_eq_does_not_crash`, gh-143543) hits the exact
    // same double-borrow panic this restructuring exists to avoid. Extract
    // the state under a SHORT borrow, do all the scanning/calling with NO
    // borrow held at all, then a second SHORT borrow to write the result
    // back.
    let is_groupby = matches!(&*args[0].borrow(), PyObject::GroupByIter { .. });
    if is_groupby {
        let (source, key_func, mut pending, exhausted) = {
            let mut obj = args[0].borrow_mut();
            if let PyObject::GroupByIter { source, key_func, pending, exhausted } = &mut *obj {
                (source.clone(), key_func.clone(), pending.take(), *exhausted)
            } else { unreachable!() }
        };
        if exhausted {
            return if args.len() >= 2 { Ok(args[1].clone()) } else { Err(PyError::stop_iteration()) };
        }
        let compute_key = |v: &PyObjectRef| -> PyResult<PyObjectRef> {
            match &key_func {
                Some(f) => call_bound_method(f.clone(), v.clone(), vec![]),
                None => Ok(v.clone()),
            }
        };
        // First item of this group: either carried over from the previous
        // call's lookahead, or freshly read from the source.
        let (this_key, first_val) = match pending.take() {
            Some((k, v)) => (k, v),
            None => {
                match builtin_next(&[source.clone()]) {
                    Ok(v) => { let k = compute_key(&v)?; (k, v) }
                    Err(PyError::StopIteration) => {
                        let mut obj = args[0].borrow_mut();
                        if let PyObject::GroupByIter { exhausted, .. } = &mut *obj { *exhausted = true; }
                        return if args.len() >= 2 { Ok(args[1].clone()) } else { Err(PyError::stop_iteration()) };
                    }
                    Err(e) => return Err(e),
                }
            }
        };
        let mut group = vec![first_val];
        let mut new_pending = None;
        let mut new_exhausted = false;
        loop {
            match builtin_next(&[source.clone()]) {
                Ok(v) => {
                    let k = compute_key(&v)?;
                    if this_key.equals(&k)? {
                        group.push(v);
                    } else {
                        new_pending = Some((k, v));
                        break;
                    }
                }
                Err(PyError::StopIteration) => { new_exhausted = true; break; }
                Err(e) => return Err(e),
            }
        }
        {
            let mut obj = args[0].borrow_mut();
            if let PyObject::GroupByIter { pending, exhausted, .. } = &mut *obj {
                *pending = new_pending;
                *exhausted = new_exhausted;
            }
        }
        return Ok(py_tuple(vec![this_key, PyObjectRef::new(PyObject::ListIter { list: group, index: 0 })]));
    }
    let mut obj = args[0].borrow_mut();
    match &mut *obj {
        PyObject::List(v) => {
            if v.is_empty() {
                if args.len() >= 2 { Ok(args[1].clone()) }
                else { Err(PyError::stop_iteration()) }
            } else {
                // Convert to ListIter for O(1) iteration
                let list = std::mem::take(v);
                *obj = PyObject::ListIter { list, index: 0 };
                drop(obj);
                let mut obj = args[0].borrow_mut();
                if let PyObject::ListIter { list, index } = &mut *obj {
                    let v = list[*index].clone();
                    *index += 1;
                    Ok(v)
                } else { unreachable!() }
            }
        }
        PyObject::ListIter { list, index } => {
            if *index >= list.len() {
                if args.len() >= 2 { Ok(args[1].clone()) }
                else { Err(PyError::stop_iteration()) }
            } else {
                let v = list[*index].clone();
                *index += 1;
                Ok(v)
            }
        }
        PyObject::CycleIter { items, index } => {
            if items.is_empty() {
                if args.len() >= 2 { Ok(args[1].clone()) }
                else { Err(PyError::stop_iteration()) }
            } else {
                let v = items[*index % items.len()].clone();
                *index += 1;
                Ok(v)
            }
        }
        PyObject::EnumerateIter { source, pos, start } => {
            // Genuinely lazy — pulls one item from the underlying `source`
            // iterator per call instead of the OLD approach (a
            // pre-materialized `items: Vec<PyObjectRef>`, built by eagerly
            // draining the whole input up front in `builtin_enumerate`).
            // That eager drain hung forever on any genuinely infinite
            // iterable (`itertools.cycle(...)`, `itertools.count()` past
            // its own internal materialization cap) — confirmed via the
            // simplest repro, `enumerate(itertools.cycle([1,2,3]))`, which
            // never even got to yield its first pair.
            let idx = *start + *pos;
            *pos += 1;
            let source = source.clone();
            drop(obj);
            match builtin_next(&[source]) {
                Ok(val) => Ok(py_tuple(vec![py_int(idx as i64), val])),
                Err(PyError::StopIteration) => {
                    if args.len() >= 2 { Ok(args[1].clone()) } else { Err(PyError::stop_iteration()) }
                }
                Err(e) => Err(e),
            }
        }
        PyObject::MapIterator { func, iterator } => {
            let iter = iterator.as_ref().clone();
            let next = builtin_next(&[iter]);
            match next {
                Ok(val) => {
                    if func.borrow().type_name() == "NoneType" {
                        Ok(val)
                    } else {
                        let mapped = builtin_call(func, &[val])?;
                        Ok(mapped)
                    }
                }
                Err(e) => {
                    if args.len() >= 2 { Ok(args[1].clone()) }
                    else { Err(e) }
                }
            }
        }
        PyObject::FilterIterator { func, iterator } => {
            let iter = iterator.as_ref().clone();
            loop {
                let next = builtin_next(&[iter.clone()]);
                match next {
                    Ok(val) => {
                        let should_keep = func.borrow().type_name() == "NoneType" || builtin_call(func, &[val.clone()])?.truthy();
                        if should_keep {
                            return Ok(val);
                        }
                    }
                    Err(e) => {
                        if args.len() >= 2 { return Ok(args[1].clone()) }
                        else { return Err(e) }
                    }
                }
            }
        }
        PyObject::ZipIterator { iterators } => {
            let mut results = Vec::with_capacity(iterators.len());
            for it in iterators.iter() {
                match builtin_next(&[it.clone()]) {
                    Ok(val) => results.push(val),
                    Err(e) => {
                        if args.len() >= 2 { return Ok(args[1].clone()) }
                        else { return Err(e) }
                    }
                }
            }
            Ok(py_tuple(results))
        }
        PyObject::RangeIter { current, stop, step } => {
            if (*step > 0 && *current >= *stop) || (*step < 0 && *current <= *stop) {
                if args.len() >= 2 { Ok(args[1].clone()) }
                else { Err(PyError::stop_iteration()) }
            } else {
                let v = py_int(*current);
                // A plain `+=` panics ("attempt to add with overflow") once
                // `current` gets within `step` of i64::MAX/MIN — real,
                // confirmed trigger: CPython's own `test_range.py` exercises
                // ranges near those boundaries. Saturating instead just
                // clamps `current` past `stop` on the affected side, which
                // correctly starves the NEXT call into `StopIteration`
                // above — the just-returned `v` here is unaffected either
                // way.
                *current = current.checked_add(*step).unwrap_or(if *step > 0 { i64::MAX } else { i64::MIN });
                Ok(v)
            }
        }
        _ => Err(PyError::type_error(format!("'{}' is not an iterator", obj.type_name()))),
    }
}

pub fn builtin_sum(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error("sum() takes at least 1 argument"));
    }
    let start = if args.len() >= 2 { args[1].clone() } else { py_int(0) };
    let mut total = start;
    let iterable = builtin_iter(&[args[0].clone()])?;
    loop {
        match builtin_next(&[iterable.clone()]) {
            Ok(val) => { total = py_add(&total, &val)?; }
            Err(PyError::StopIteration) => return Ok(total),
            Err(e) => return Err(e),
        }
    }
}

fn compare_gt(a: &PyObjectRef, b: &PyObjectRef) -> std::cmp::Ordering {
    // Route through py_compare so user-defined classes' __gt__/__lt__ are
    // consulted (the raw Compare trait has no notion of Instance dispatch).
    match py_compare(a, b, 4) {
        Ok(result) if result.truthy() => std::cmp::Ordering::Greater,
        _ => std::cmp::Ordering::Less,
    }
}

pub fn builtin_max(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() { return Err(PyError::type_error("max() requires at least 1 argument")); }
    let items: Vec<PyObjectRef> = if args.len() == 1 {
        let mut v = Vec::new();
        let iterable = builtin_iter(&[args[0].clone()])?;
        loop {
            match builtin_next(&[iterable.clone()]) {
                Ok(val) => v.push(val),
                Err(PyError::StopIteration) => break,
                Err(e) => return Err(e),
            }
        }
        v
    } else {
        args.to_vec()
    };
    items.into_iter().max_by(compare_gt).ok_or_else(|| PyError::value_error("max() arg is an empty sequence"))
}

pub fn builtin_min(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() { return Err(PyError::type_error("min() requires at least 1 argument")); }
    let items: Vec<PyObjectRef> = if args.len() == 1 {
        let mut v = Vec::new();
        let iterable = builtin_iter(&[args[0].clone()])?;
        loop {
            match builtin_next(&[iterable.clone()]) {
                Ok(val) => v.push(val),
                Err(PyError::StopIteration) => break,
                Err(e) => return Err(e),
            }
        }
        v
    } else {
        args.to_vec()
    };
    items.into_iter().min_by(compare_gt).ok_or_else(|| PyError::value_error("min() arg is an empty sequence"))
}

pub fn builtin_id(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() != 1 {
        return Err(PyError::type_error("id() takes exactly one argument"));
    }
    Ok(py_int(args[0].get_id() as i64))
}

pub fn builtin_vars(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() != 1 {
        return Err(PyError::type_error("vars() takes exactly one argument"));
    }
    let obj = args[0].borrow();
    match &*obj {
        PyObject::Instance { dict, .. } => {
            let mut pd = PyDict::new();
            for (k, v) in dict.iter() {
                pd.set(py_str(k), v.clone())?;
            }
            Ok(PyObjectRef::new(PyObject::Dict(Box::new(pd))))
        }
        PyObject::Module { dict, .. } => {
            let mut pd = PyDict::new();
            for (k, v) in dict.iter() {
                pd.set(py_str(interner::lookup_str(*k)), v.clone())?;
            }
            Ok(PyObjectRef::new(PyObject::Dict(Box::new(pd))))
        }
        _ => Err(PyError::type_error(format!("vars() argument must have __dict__ attribute"))),
    }
}

thread_local! {
    // `isinstance()`/`issubclass()` recurse in PLAIN RUST (calling
    // themselves directly, not through `call_function`) when the
    // `classinfo` argument is a tuple that itself contains tuples —
    // arbitrarily deeply, per real Python semantics (`isinstance(x, (a,
    // (b, (c, ...))))`). `vm.rs`'s `call_function` recursion-depth guard
    // (see its own doc comment) only covers actual Python-level function
    // calls, not this native-Rust recursion, so a deeply/infinitely nested
    // classinfo tuple had no depth limit at all — confirmed general via
    // CPython's own `test_isinstance.py`'s `blowstack()` helper, which
    // builds an ever-growing nested tuple in a `while True:` loop
    // expecting `RecursionError` to interrupt it "eventually": without a
    // guard here, it never does, hanging forever instead of failing fast.
    static ISINSTANCE_RECURSION_DEPTH: std::cell::Cell<usize> = std::cell::Cell::new(0);
}

struct IsinstanceRecursionGuard;

impl IsinstanceRecursionGuard {
    fn enter() -> PyResult<Self> {
        let depth = ISINSTANCE_RECURSION_DEPTH.with(|c| {
            let d = c.get() + 1;
            c.set(d);
            d
        });
        if depth > 1000 {
            ISINSTANCE_RECURSION_DEPTH.with(|c| c.set(c.get() - 1));
            return Err(PyError::recursion_error("maximum recursion depth exceeded"));
        }
        Ok(IsinstanceRecursionGuard)
    }
}

impl Drop for IsinstanceRecursionGuard {
    fn drop(&mut self) {
        ISINSTANCE_RECURSION_DEPTH.with(|c| c.set(c.get().saturating_sub(1)));
    }
}

pub fn builtin_isinstance(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() != 2 {
        return Err(PyError::type_error("isinstance() takes exactly 2 arguments"));
    }
    let obj = args[0].borrow();
    let class = args[1].borrow();
    // Handle tuple of types: isinstance(x, (type1, type2, ...))
    if let PyObject::Tuple(types) = &*class {
        let _guard = IsinstanceRecursionGuard::enter()?;
        for t in types {
            let check_args = vec![args[0].clone(), t.clone()];
            if builtin_isinstance(&check_args)?.truthy() {
                return Ok(py_bool(true));
            }
        }
        return Ok(py_bool(false));
    }
    match (&*obj, &*class) {
        (PyObject::Type { .. }, PyObject::Type { name: class_name, .. }) => {
            // isinstance(SomeClass, X): every class is an instance of its
            // metaclass (a custom one if it was built via `metaclass=`,
            // otherwise plain `type` — e.g. `isinstance(Foo, type)` is
            // True for any ordinary class `Foo`). Walk the metaclass's own
            // mro for `X`, exactly like the Instance case above walks the
            // *class's* mro for ordinary instance checks. Deliberately
            // avoids fetching the canonical `type`/`object` singletons via
            // `with_vm_mut` for the no-custom-metaclass fallback below —
            // `isinstance()` runs deep inside live call chains constantly,
            // and grabbing a second aliasing `&mut VirtualMachine` there
            // reliably segfaulted in testing; a plain name comparison
            // (matching how BuiltinFunction-represented native bases are
            // already recognized elsewhere in this function) avoids it.
            if let Some(mt) = metatype_of(&args[0]) {
                if mt.is(&args[1]) {
                    return Ok(py_bool(true));
                }
                if let PyObject::Type { mro, .. } = &*mt.borrow() {
                    for c in mro {
                        if c.is(&args[1]) {
                            return Ok(py_bool(true));
                        }
                    }
                }
                return Ok(py_bool(false));
            }
            Ok(py_bool(class_name == "type" || class_name == "object"))
        }
        (PyObject::Instance { typ, .. }, PyObject::Type { .. }) => {
            // isinstance(obj, Cls): Cls must appear in obj's OWN type's mro
            // — this used to walk `Cls`'s mro checking for `obj`'s exact
            // class name instead (backwards: comparing the wrong object's
            // ancestry, and by name instead of identity), so
            // `isinstance(subclass_instance, ParentClass)` was always False
            // unless the instance's class name happened to literally match
            // one of Cls's own ancestors' names. Confirmed via a minimal,
            // Django-free repro: `isinstance(CharField(), Field)` (a plain
            // one-level subclass check) returning False — this broke every
            // `isinstance(other, Field)`-style guard in real code (Django's
            // `Field.__lt__`/`__eq__`, used by `@total_ordering` and
            // `bisect`-based field ordering during model construction).
            let typ_ref = typ.borrow();
            if let PyObject::Type { mro, .. } = &*typ_ref {
                for c in mro {
                    if c.is(&args[1]) {
                        return Ok(py_bool(true));
                    }
                }
            }
            drop(typ_ref);
            // `ABCMeta.register(subclass)`-style virtual subclass checks
            // (see `.register`'s own doc comment, `get_attribute`'s
            // `PyObject::Type` arm) — `class` (the ABC) records registered
            // classes in its own `_abc_registry` frozenset (read via
            // `own_abc_registry`, NOT `get_attribute` — see its own doc
            // comment for why inherited/MRO-walked registries are wrong
            // here); `obj`'s class (or any of ITS ancestors, for a real
            // subclass of a registered virtual-subclass root) counts too.
            if abc_registry_matches_in_subtree(&args[1], &|registered| {
                typ.is(registered) || matches!(&*typ.borrow(), PyObject::Type { mro, .. } if mro.iter().any(|c| c.is(registered)))
            }) {
                return Ok(py_bool(true));
            }
            Ok(py_bool(false))
        }
        (PyObject::Instance { typ, .. }, _) => {
            let class_name = match &*class {
                PyObject::BuiltinFunction { name, .. } => name.clone(),
                PyObject::Str(s) => s.to_string(),
                _ => class.str(),
            };
            // `class Foo(list): ...` — Foo transparently subclasses a
            // native builtin, so isinstance(foo, list) must also be true.
            if let Some(native_kind) = native_base_of_type(typ) {
                if native_kind == class_name {
                    return Ok(py_bool(true));
                }
            }
            // `class MyError(Exception): ...` — MyError's instances must
            // also be `isinstance(x, Exception)` (and `AttributeError`,
            // `BaseException`, etc, for whatever it really derives from).
            // Builtin exception "classes" are `PyObject::BuiltinFunction`s
            // that never appear in a custom class's own `mro` (only real
            // `PyObject::Type` bases do), so the mro-walk arm just above
            // can't see this relationship at all — without this, NO custom
            // exception subclass was ever recognized as an instance of any
            // of its real (builtin) ancestors, which also broke `except
            // Exception:`/`except SomeBuiltinBase:` for literally any
            // user-defined exception class (CHECK_EXC_MATCH, `vm.rs`, and
            // `builtin_issubclass` below share this exact same gap and fix).
            if let PyObject::BuiltinFunction { .. } = &*class {
                if let Some(base_name) = find_exception_base_name(typ) {
                    if crate::vm::is_exception_subclass(&base_name, &class_name) {
                        return Ok(py_bool(true));
                    }
                }
            }
            Ok(py_bool(typ.borrow().type_name() == class_name || class_name == "object"))
        }
        // `isinstance(TypeError, type)` (and similar for any builtin
        // exception "class") — real CPython: every exception class's
        // metaclass is `type`, so this must be True. Since builtin
        // exception classes are represented as plain `PyObject::BuiltinFunction`s
        // here (not `PyObject::Type`s), the generic name-based hierarchy
        // check in the catch-all arm below instead compared this
        // function's own native `type_name()` ("builtin_function_or_method")
        // against "type" in the EXCEPTION hierarchy table (nonsensical —
        // that table is for exception ANCESTRY, not metaclass checks) and
        // always returned False. Real trigger: `unittest`'s own
        // `case.py`'s `_is_subtype`, `isinstance(expected, type) and
        // issubclass(expected, basetype)` — used by `assertRaises(SomeErr)`
        // to validate its argument — always failed, making
        // `self.assertRaises(TypeError)` (or ANY builtin exception class)
        // raise `TypeError: assertRaises() arg 1 must be an exception type
        // or tuple of exception types` instead of actually working.
        (PyObject::BuiltinFunction { name, .. }, PyObject::Type { name: cname, .. })
            if is_builtin_exception_class_name(name) =>
        {
            Ok(py_bool(cname == "type" || cname == "object"))
        }
        _ => {
            let obj_type = args[0].borrow().type_name();
            let class_name = match &*class {
                PyObject::BuiltinFunction { name, .. } => name.clone(),
                PyObject::Str(s) => s.to_string(),
                PyObject::Type { name, .. } => name.clone(),
                _ => class.str(),
            };
            // Direct type name match for built-in types (int, str, list, etc.)
            if obj_type == class_name {
                return Ok(py_bool(true));
            }
            // `bool` is real CPython's one primitive with an actual
            // inheritance relationship to another primitive
            // (`bool.__bases__ == (int,)`) — `isinstance(True, int)` and
            // `isinstance(True, object)` must both be `True`. A `bool`
            // value here is always the PRIMITIVE `PyObjectRef::SmallBool`/
            // `PyObject::Bool` shape (never a `PyObject::Instance`, since
            // `bool` can't be subclassed — see `default_build_class`'s
            // explicit block for that), so this can't be expressed via the
            // generic mro-walk arms above (which all key off an `Instance`'s
            // `typ`) — a narrow, direct name check is the simplest correct
            // fix, mirroring this function's existing style for other
            // primitive/name-based special cases.
            if obj_type == "bool" && (class_name == "int" || class_name == "object") {
                return Ok(py_bool(true));
            }
            // See `builtin_isinstance`'s other `_abc_registry` fallback —
            // this one covers a PRIMITIVE (inline `SmallInt`/`SmallStr`/...
            // or boxed `Int`/`Str`/...) checked against an ABC that
            // registered the matching builtin type name (real trigger:
            // `numbers.py`'s own `Integral.register(int)`/`Real.register
            // (float)`/`Complex.register(complex)` — needed for
            // `isinstance(5, numbers.Integral)` to work at all).
            if let PyObject::Type { .. } = &*class {
                if abc_registry_matches_in_subtree(&args[1], &|registered| {
                    let registered_name = match &*registered.borrow() {
                        PyObject::BuiltinFunction { name, .. } => name.clone(),
                        PyObject::Type { name, .. } => name.clone(),
                        _ => return false,
                    };
                    registered_name == obj_type
                }) {
                    return Ok(py_bool(true));
                }
            }
            // Exception hierarchy
            Ok(py_bool(crate::vm::is_exception_subclass(&obj_type, &class_name)))
        }
    }
}

/// Real `open()`/`os.*` path arguments accept `str`, `bytes`, or any
/// `os.PathLike` — but `PyObjectRef::str()` on a `PyObject::Bytes` value
/// deliberately returns Python's own `repr()`-style `"b'...'"` form (matching
/// real CPython's `str(b"x")` semantics — NOT a bug on its own), which is
/// exactly the WRONG thing to feed to a filesystem call. Real trigger:
/// CPython's own `dbm/dumb.py` (via `os.fsencode`), which builds its
/// `.dat`/`.dir`/`.bak` filenames as `bytes` and passes them straight to
/// `io.open()` — `open(b"/tmp/x.dat", "w")` tried to open a file literally
/// named `b'/tmp/x.dat'` (quotes, `b` prefix and all), which obviously
/// doesn't exist, raising a confusing `OSError` instead of writing to the
/// intended path.
pub(crate) fn path_arg_to_string(obj: &PyObjectRef) -> String {
    if let PyObject::Bytes(b) = &*obj.borrow() {
        String::from_utf8_lossy(b).into_owned()
    } else {
        obj.str()
    }
}

pub fn builtin_open(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error("open() missing required argument 'file'"));
    }
    let filename = path_arg_to_string(&args[0]);
    let mode = if args.len() > 1 { args[1].str() } else { "r".to_string() };
    // A trailing `+` ("r+"/"w+"/"a+", real CPython's "and updating" suffix)
    // means the file is opened for BOTH reading and writing — was
    // completely ignored here, so "rb+" (read-write, don't truncate, don't
    // create — the exact mode `dbm/dumb.py` uses to append new values to
    // its own data file) only ever opened for reading, and a subsequent
    // `f.write(...)` failed with a raw OS-level "Bad file descriptor"
    // instead of writing.
    let has_plus = mode.contains('+');
    let file = std::fs::File::options()
        .read(mode.contains('r') || has_plus)
        .write(mode.contains('w') || mode.contains('a') || has_plus)
        .append(mode.contains('a'))
        .create(mode.contains('w') || mode.contains('a'))
        .truncate(mode.contains('w'))
        .open(&filename)
        .map_err(|e| PyError::OsError(format!("{}", e)))?;
    Ok(PyObjectRef::new(PyObject::File { file: std::rc::Rc::new(std::cell::RefCell::new(file)), name: filename }))
}

pub fn builtin_any(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error("any() requires at least 1 argument"));
    }
    let iterable = builtin_iter(&[args[0].clone()])?;
    loop {
        match builtin_next(&[iterable.clone()]) {
            Ok(val) => if val.truthy() { return Ok(py_bool(true)); },
            Err(PyError::StopIteration) => return Ok(py_bool(false)),
            Err(e) => return Err(e),
        }
    }
}

pub fn builtin_all(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error("all() requires at least 1 argument"));
    }
    let iterable = builtin_iter(&[args[0].clone()])?;
    loop {
        match builtin_next(&[iterable.clone()]) {
            Ok(val) => if !val.truthy() { return Ok(py_bool(false)); },
            Err(PyError::StopIteration) => return Ok(py_bool(true)),
            Err(e) => return Err(e),
        }
    }
}

pub fn builtin_callable(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() != 1 {
        return Err(PyError::type_error("callable() takes exactly one argument"));
    }
    let obj = args[0].borrow();
    let is_callable = matches!(&*obj,
        PyObject::Function(_) | PyObject::BuiltinFunction { .. } |
        PyObject::BuiltinMethod { .. } | PyObject::Type { .. } | PyObject::BuildClass |
        PyObject::BoundMethod { .. } | PyObject::Partial { .. } |
        PyObject::Generator { .. } | PyObject::Coroutine { .. } |
        // Instances may be callable if they have __call__
        PyObject::Instance { .. }
    );
    // For instances, check if the type (or a base, via MRO) has __call__
    if !is_callable {
        Ok(py_bool(false))
    } else if let PyObject::Instance { typ, .. } = &*obj {
        Ok(py_bool(lookup_dunder_via_mro(typ, "__call__").is_some()))
    } else {
        Ok(py_bool(true))
    }
}

pub fn builtin_breakpoint(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if !args.is_empty() {
        eprintln!(
            "Breakpoint reached with {} argument(s) — debugger not available in this interpreter",
            args.len()
        );
        for (i, arg) in args.iter().enumerate() {
            eprintln!("  arg[{}]: {}", i, arg.str());
        }
    } else {
        eprintln!("Breakpoint reached — debugger not available in this interpreter");
    }
    Ok(py_none())
}

pub fn builtin_pow(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 2 {
        return Err(PyError::type_error("pow() requires at least 2 arguments"));
    }
    let result = py_pow(&args[0], &args[1])?;
    if args.len() == 3 {
        py_mod(&result, &args[2])
    } else {
        Ok(result)
    }
}

pub fn builtin_reversed(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() != 1 {
        return Err(PyError::type_error("reversed() takes exactly one argument"));
    }
    // Check type with a short-lived borrow to avoid holding the RefCell
    // borrow while iterating (which could trigger borrow_mut conflicts).
    // `range` needs its own O(1) case (real CPython's `range.__reversed__`)
    // — without this it fell into the generic "drain every element into a
    // Vec, then reverse" fallback further down, which for a `range` spanning
    // billions of elements tries to materialize the WHOLE thing first. Same
    // unbounded-incremental-growth bug as the `list()`/`list * n` memory
    // bombs fixed elsewhere (confirmed via CPython's own `test_range.py`,
    // `test_range_iterators`, whose `reversed(range(start, end, step))`
    // calls span ranges up to ~2**33 elements — enough to consume all
    // available RAM before ever finishing). `range`'s length is always
    // O(1) to compute, so the reversed sequence can be derived directly,
    // arithmetically, without ever iterating the original.
    {
        let obj = args[0].borrow();
        if let PyObject::Range { start, stop, step } = &*obj {
            let (start, stop, step) = (*start, *stop, *step);
            let empty = (step > 0 && start >= stop) || (step < 0 && start <= stop);
            if empty {
                return Ok(PyObjectRef::new(PyObject::RangeIter { current: 0, stop: 0, step: 1 }));
            }
            let raw_len = (stop as i128) - (start as i128);
            let step128 = step as i128;
            let q = raw_len / step128;
            let count: i128 = if raw_len % step128 != 0 { q.abs() + 1 } else { q.abs() };
            let last = (start as i128 + (count - 1) * step128) as i64;
            let new_stop = start.wrapping_sub(step);
            return Ok(PyObjectRef::new(PyObject::RangeIter { current: last, stop: new_stop, step: -step }));
        }
    }
    let kind = {
        let obj = args[0].borrow();
        match &*obj {
            PyObject::List(_) => 1,
            PyObject::Tuple(_) => 2,
            PyObject::Str(_) => 3,
            _ => 0,
        }
    };
    if kind != 0 {
        let obj = args[0].borrow();
        return match &*obj {
            PyObject::List(v) => {
                let mut rev = v.clone(); rev.reverse();
                Ok(PyObjectRef::new(PyObject::ListIter { list: rev, index: 0 }))
            }
            PyObject::Tuple(v) => {
                let mut rev = v.clone(); rev.reverse();
                Ok(PyObjectRef::new(PyObject::ListIter { list: rev, index: 0 }))
            }
            PyObject::Str(s) => {
                let chars: Vec<PyObjectRef> = s.chars().rev().map(|c| py_str(&c.to_string())).collect();
                Ok(PyObjectRef::new(PyObject::ListIter { list: chars, index: 0 }))
            }
            _ => unreachable!(),
        };
    }
    // Unknown type: drain via iteration (no active borrow on args[0])
    let mut v = Vec::new();
    let iterable = builtin_iter(&[args[0].clone()])?;
    loop {
        match builtin_next(&[iterable.clone()]) {
            Ok(val) => v.push(val),
            Err(PyError::StopIteration) => break,
            Err(e) => return Err(e),
        }
    }
    v.reverse();
    Ok(PyObjectRef::new(PyObject::ListIter { list: v, index: 0 }))
}

pub fn builtin_issubclass(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() != 2 {
        return Err(PyError::type_error("issubclass() takes exactly 2 arguments"));
    }
    // Handle tuple of types: issubclass(cls, (type1, type2, ...))
    let base = args[1].borrow();
    if let PyObject::Tuple(types) = &*base {
        let _guard = IsinstanceRecursionGuard::enter()?;
        for t in types {
            let check_args = vec![args[0].clone(), t.clone()];
            if builtin_issubclass(&check_args)?.truthy() {
                return Ok(py_bool(true));
            }
        }
        return Ok(py_bool(false));
    }
    let cls = args[0].borrow();
    drop(base);
    let base = args[1].borrow();
    match (&*cls, &*base) {
        (PyObject::Type { mro: cls_mro, .. }, PyObject::Type { .. }) => {
            let base_tn = base.type_name();
            for c in cls_mro {
                if c.borrow().type_name() == base_tn {
                    return Ok(py_bool(true));
                }
            }
            // See `builtin_isinstance`'s matching fallback for
            // `ABCMeta.register`-style virtual subclass checks.
            if abc_registry_matches_in_subtree(&args[1], &|registered| cls_mro.iter().any(|c| c.is(registered))) {
                return Ok(py_bool(true));
            }
            Ok(py_bool(false))
        }
        (PyObject::Type { mro: cls_mro, .. }, _) => {
            // Non-Type second argument: compare by name. `.str()` on a
            // BuiltinFunction (how builtin exception "classes" like
            // `Exception` are represented) returns its full repr
            // (`<built-in function Exception>`), not the bare name — must
            // extract that explicitly or every name comparison below
            // (including the exception-ancestry fix just past the mro
            // walk) silently never matches.
            let base_name = match &*base {
                PyObject::BuiltinFunction { name, .. } => name.clone(),
                _ => base.str(),
            };
            if base_name == "object" {
                return Ok(py_bool(true));
            }
            for c in cls_mro {
                if c.borrow().type_name() == base_name {
                    return Ok(py_bool(true));
                }
            }
            // `issubclass(MyError, Exception)` where MyError is a real
            // user-defined `class MyError(Exception): ...` — Exception (a
            // `BuiltinFunction`, not a `Type`) never appears in MyError's own
            // `mro`, so the walk above can't see this. Same gap/fix as
            // isinstance()'s Instance/BuiltinFunction arm just above.
            if matches!(&*base, PyObject::BuiltinFunction { .. }) {
                if let Some(base_exc_name) = find_exception_base_name(&args[0]) {
                    if crate::vm::is_exception_subclass(&base_exc_name, &base_name) {
                        return Ok(py_bool(true));
                    }
                }
            }
            Ok(py_bool(false))
        }
        // Built-in exception "classes" (e.g. KeyError, ValueError) are
        // represented as BuiltinFunction constructors rather than Type
        // objects — resolve ancestry by name via the same table `except`
        // and isinstance() use, instead of only accepting real Type values.
        (PyObject::BuiltinFunction { name: cls_name, .. }, _) => {
            let base_name = match &*base {
                PyObject::BuiltinFunction { name, .. } => name.clone(),
                PyObject::Str(s) => s.to_string(),
                PyObject::Type { name, .. } => name.clone(),
                _ => base.str(),
            };
            if crate::vm::is_exception_subclass(cls_name, &base_name) {
                return Ok(py_bool(true));
            }
            // See `builtin_isinstance`'s `_abc_registry` fallback — this
            // covers `issubclass(int, numbers.Integral)`-style checks
            // where the SUBCLASS side (`int`/`float`/`complex`/...) is
            // itself a `BuiltinFunction`, not a `Type`. Real trigger:
            // `numbers.py`'s own `Integral.register(int)`/`Real.register
            // (float)`/`Complex.register(complex)`.
            if let PyObject::Type { .. } = &*base {
                if abc_registry_matches_in_subtree(&args[1], &|registered| {
                    let registered_name = match &*registered.borrow() {
                        PyObject::BuiltinFunction { name, .. } => name.clone(),
                        PyObject::Type { name, .. } => name.clone(),
                        _ => return false,
                    };
                    &registered_name == cls_name
                }) {
                    return Ok(py_bool(true));
                }
            }
            Ok(py_bool(false))
        }
        _ => {
            if std::env::var("RPY_DEBUG_ISSUBCLASS").is_ok() {
                eprintln!("issubclass() FAIL: arg0={:?}/{} arg1={:?}/{}", cls.type_name(), cls.repr(), base.type_name(), base.repr());
            }
            Err(PyError::type_error("issubclass() arg 1 must be a class"))
        }
    }
}

pub fn builtin_help(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        println!("Welcome to RustPython 0.1.0!");
        println!();
        println!("Available built-in functions:");
        println!("  abs()  all()  any()  ascii()  bin()  bool()  breakpoint()");
        println!("  bytearray()  bytes()  callable()  chr()  compile()  delattr()");
        println!("  dict()  dir()  divmod()  enumerate()  eval()  exec()  exit()");
        println!("  filter()  float()  format()  frozenset()  getattr()  globals()");
        println!("  hasattr()  hash()  help()  hex()  id()  input()  int()");
        println!("  isinstance()  issubclass()  iter()  len()  list()  locals()");
        println!("  map()  max()  memoryview()  min()  next()  object()  oct()");
        println!("  open()  ord()  pow()  print()  property()  range()  repr()");
        println!("  reversed()  round()  set()  setattr()  slice()  sorted()");
        println!("  staticmethod()  str()  sum()  super()  tuple()  type()  vars()");
        println!("  zip()");
        println!();
        println!("Available error types:");
        println!("  BaseException  Exception  TypeError  ValueError  ZeroDivisionError");
        println!("  NameError  AttributeError  IndexError  KeyError  RuntimeError");
        println!("  StopIteration  AssertionError  OSError  ImportError  LookupError");
        println!("  ArithmeticError  OverflowError  NotImplementedError  RecursionError");
        println!("  KeyboardInterrupt  SystemExit  ModuleNotFoundError  FileNotFoundError");
        println!("  PermissionError  UnicodeDecodeError  UnicodeEncodeError");
        println!();
        println!("Type help(object) for information about a specific object.");
    } else {
        let obj = args[0].borrow();
        match &*obj {
            PyObject::Type { name, dict, .. } => {
                println!("Help on class {}:", name);
                if let Some(doc) = dict.get_str("__doc__") {
                    println!("  {}", doc.str());
                }
                println!();
                println!("Methods:");
                for (key, val) in dict.iter() {
                    if matches!(&*val.borrow(), PyObject::Function(_) | PyObject::BuiltinFunction { .. }) {
                        println!("  {}()", interner::lookup_str(*key));
                    }
                }
            }
            PyObject::Function(ref f) => {
            let name = &f.code.name;
            let dict = &f.dict;
                println!("Help on function {}:", name);
                if let Some(doc) = dict.get("__doc__") {
                    println!("  {}", doc.str());
                }
            }
            PyObject::BuiltinFunction { name, .. } => {
                println!("Help on built-in function {}:", name);
            }
            _ => {
                println!("Help on {}:", obj.type_name());
                println!("  Type: {}", obj.type_name());
            }
        }
    }
    Ok(py_none())
}

