// Split out of the former monolithic object/builtins.rs — this file holds
// attribute / introspection-related builtins that operate on object attributes
// (`hasattr`, `getattr`, `setattr`, `delattr`) and small conversion helpers
// (`ord`, `chr`, `hex`, `oct`, `bin`, `ascii`, `input`, `exit`).
use super::*;

pub fn builtin_hasattr(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() != 2 {
        return Err(PyError::type_error("hasattr() takes exactly 2 arguments"));
    }
    let attr_name = args[1].str();
    if std::env::var("RPY_DEBUG_GETATTR").is_ok() {
        eprintln!(
            "HASATTR: obj_type={} attr={}",
            args[0].borrow().type_name(),
            attr_name
        );
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
    if let PyObject::WeakProxy { target, .. } = &*args[0].borrow() {
        eprintln!("builtin_setattr WeakProxy");
        if let Some(rc) = target.upgrade() {
            let target_ref = PyObjectRef::Mut(rc);
            return builtin_setattr(&[target_ref, args[1].clone(), args[2].clone()]);
        } else {
            return Err(PyError::reference_error(
                "weakly-referenced object no longer exists",
            ));
        }
    }
    let attr_name = args[1].str();
    // Check for __set__ descriptor protocol on data descriptors first
    // (property/DynamicClassAttribute must intercept before plain
    // instance-dict assignment, matching LOAD_ATTR/STORE_ATTR opcode paths).
    let descriptor = {
        let obj_borrowed = args[0].borrow();
        if let PyObject::Instance { typ, .. } = &*obj_borrowed {
            let typ_ref = typ.borrow();
            if let PyObject::Type {
                dict: type_dict, ..
            } = &*typ_ref
            {
                type_dict.get_str(&attr_name).cloned()
            } else {
                None
            }
        } else {
            None
        }
    };
    if let Some(ref desc) = descriptor {
        // Property is special-cased: call the setter function via the
        // live VM (not a disposable one) — matches STORE_ATTR opcode
        // path and avoids import/global lookups failing.
        let is_property = {
            let d = desc.borrow();
            matches!(&*d, PyObject::Property(_))
        };
        if is_property {
            let property_setter = {
                let d = desc.borrow();
                if let PyObject::Property(ref data) = &*d {
                    data.setter.clone()
                } else {
                    None
                }
            };
            if let Some(setter_fn) = property_setter {
                return with_vm_mut(|vm| {
                    vm.call_function(setter_fn, vec![args[0].clone(), args[2].clone()], vec![])
                })?;
            } else {
                return Err(PyError::attribute_error(format!(
                    "property '{}' of '{}' object has no setter",
                    attr_name,
                    args[0].borrow().type_name()
                )));
            }
        }
        // Generic __set__ for other descriptors (DynamicClassAttribute, etc.)
        if let Ok(setter) = desc.borrow().get_attribute("__set__") {
            return with_vm_mut(|vm| {
                vm.call_function(
                    setter,
                    vec![desc.clone(), args[0].clone(), args[2].clone()],
                    vec![],
                )
            })?;
        }
    }
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
            "'{}' object has no attribute '{}'",
            args[0].borrow().type_name(),
            attr_name
        )));
    }
    args[0]
        .borrow_mut()
        .set_attribute(&attr_name, args[2].clone())?;
    Ok(py_none())
}

pub fn builtin_delattr(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() != 2 {
        return Err(PyError::type_error("delattr() takes exactly 2 arguments"));
    }
    let attr_name = args[1].str();
    // Check for __delete__ descriptor protocol first (data descriptors
    // like property/DynamicClassAttribute must intercept deletion before
    // __delattr__, matching how builtin_setattr checks __set__ first).
    let descriptor = {
        let obj_borrowed = args[0].borrow();
        if let PyObject::Instance { typ, .. } = &*obj_borrowed {
            let typ_ref = typ.borrow();
            if let PyObject::Type {
                dict: type_dict, ..
            } = &*typ_ref
            {
                type_dict.get_str(&attr_name).cloned()
            } else {
                None
            }
        } else {
            None
        }
    };
    if let Some(ref desc) = descriptor {
        let is_property = {
            let d = desc.borrow();
            matches!(&*d, PyObject::Property(_))
        };
        if is_property {
            let property_deleter = {
                let d = desc.borrow();
                if let PyObject::Property(ref data) = &*d {
                    data.deleter.clone()
                } else {
                    None
                }
            };
            if let Some(del_fn) = property_deleter {
                return with_vm_mut(|vm| vm.call_function(del_fn, vec![args[0].clone()], vec![]))?;
            } else {
                return Err(PyError::attribute_error(format!(
                    "property '{}' of '{}' object has no deleter",
                    attr_name,
                    args[0].borrow().type_name()
                )));
            }
        }
        // Generic __delete__ for other descriptors (DynamicClassAttribute, etc.)
        if let Ok(deleter) = desc.borrow().get_attribute("__delete__") {
            return with_vm_mut(|vm| {
                vm.call_function(deleter, vec![desc.clone(), args[0].clone()], vec![])
            })?;
        }
    }
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
            "'{}' object has no attribute '{}'",
            args[0].borrow().type_name(),
            attr_name
        )));
    }
    args[0].borrow_mut().del_attribute(&attr_name)?;
    Ok(py_none())
}

pub fn builtin_ord(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() != 1 {
        return Err(PyError::type_error("ord() takes exactly one argument"));
    }
    // Handle bytes/bytearray of length 1 (ord(b'x') == 120)
    {
        let b = args[0].borrow();
        match &*b {
            PyObject::Bytes(v) => {
                if v.len() == 1 {
                    return Ok(py_int(v[0] as i64));
                } else {
                    return Err(PyError::type_error(format!(
                        "ord() expected a character, but string of length {} found",
                        v.len()
                    )));
                }
            }
            PyObject::ByteArray(v) => {
                if v.len() == 1 {
                    return Ok(py_int(v[0] as i64));
                } else {
                    return Err(PyError::type_error(format!(
                        "ord() expected a character, but string of length {} found",
                        v.len()
                    )));
                }
            }
            PyObject::Str(s) => {
                // Use WTF-8 codepoints to handle surrogates correctly.
                let cps: Vec<u32> = if s.has_surrogates() {
                    crate::object::pystring::Wtf8Chars::new(s.as_bytes()).collect()
                } else {
                    s.as_str().chars().map(|c| c as u32).collect()
                };
                let c = cps.first().copied().ok_or_else(|| {
                    PyError::type_error("ord() expected a character, but string of length 0 found")
                })?;
                if cps.len() != 1 {
                    return Err(PyError::type_error(format!(
                        "ord() expected a character, but string of length {} found",
                        cps.len()
                    )));
                }
                return Ok(py_int(c as i64));
            }
            _ => {}
        }
    }
    let s = args[0].str();
    let c = s.chars().next().ok_or_else(|| {
        PyError::type_error("ord() expected a character, but string of length 0 found")
    })?;
    Ok(py_int(c as u32 as i64))
}

pub fn builtin_chr(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() != 1 {
        return Err(PyError::type_error("chr() takes exactly one argument"));
    }
    let n = to_index(&args[0])?;
    let code = n
        .to_usize()
        .ok_or_else(|| PyError::value_error("chr() arg not in range(0x110000)"))?;
    // Lone surrogates (U+D800–U+DFFF): valid Python `chr()` results (Python
    // strings allow them) but Rust's `char::from_u32` rejects them — build a
    // WTF-8 PyStr directly instead (CPython: `chr(0xD800)` == `'\ud800'`).
    if (0xD800..=0xDFFF).contains(&code) {
        return Ok(py_str_pstr(&PyStr::from_surrogate(code as u32)));
    }
    let c = char::from_u32(code as u32)
        .ok_or_else(|| PyError::value_error("chr() arg not in range(0x110000)"))?;
    Ok(py_str(&c.to_string()))
}

pub fn builtin_hex(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() != 1 {
        return Err(PyError::type_error("hex() takes exactly one argument"));
    }
    let n = to_index(&args[0])?;
    let digits = format!("{:x}", n.magnitude());
    if n.sign() == num_bigint::Sign::Minus {
        Ok(py_str(&format!("-0x{}", digits)))
    } else {
        Ok(py_str(&format!("0x{}", digits)))
    }
}

pub fn builtin_oct(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() != 1 {
        return Err(PyError::type_error("oct() takes exactly one argument"));
    }
    let n = to_index(&args[0])?;
    let digits = format!("{:o}", n.magnitude());
    if n.sign() == num_bigint::Sign::Minus {
        Ok(py_str(&format!("-0o{}", digits)))
    } else {
        Ok(py_str(&format!("0o{}", digits)))
    }
}

pub fn builtin_bin(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() != 1 {
        return Err(PyError::type_error("bin() takes exactly one argument"));
    }
    let n = to_index(&args[0])?;
    let digits = format!("{:b}", n.magnitude());
    if n.sign() == num_bigint::Sign::Minus {
        Ok(py_str(&format!("-0b{}", digits)))
    } else {
        Ok(py_str(&format!("0b{}", digits)))
    }
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

pub fn builtin_input(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if !args.is_empty() {
        print!("{}", args[0].str());
    }
    let mut line = String::new();
    std::io::stdin()
        .read_line(&mut line)
        .map_err(|e| PyError::runtime_error(e.to_string()))?;
    if line.ends_with('\n') {
        line.pop();
    }
    Ok(py_str(&line))
}

pub fn builtin_exit(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::SystemExit(0));
    }
    if let Some(i) = args[0].as_i64() {
        return Err(PyError::SystemExit(i as i32));
    }
    if matches!(&*args[0].borrow(), PyObject::None) {
        return Err(PyError::SystemExit(0));
    }
    // Non-int / non-None: raise SystemExit with the object so the
    // top-level handler can print it and exit 1 (matches CPython's
    // sys.exit("error") behaviour and fixes exit(False)/exit(True)).
    Err(PyError::Exception(
        "SystemExit".to_string(),
        PyObjectRef::new(PyObject::Exception {
            typ: "SystemExit".to_string(),
            args: vec![args[0].clone()],
            cause: None,
            suppress_context: false,
            context: None,
            traceback: None,
            extra: None,
        }),
    ))
}
