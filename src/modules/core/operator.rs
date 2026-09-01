use crate::object::*;
use std::collections::HashMap;
use std::sync::atomic::AtomicI64;
use num_traits::{Signed, ToPrimitive};
use std::rc::Rc;


/// Looks up `name` on `obj` the same way the VM's own `LOAD_ATTR` opcode
/// does — as opposed to the raw `get_attribute()` free function, which does
/// NOT auto-bind. Two real gaps this closes for any caller (like
/// `attrgetter`/`methodcaller` below) that resolves an attribute
/// PROGRAMMATICALLY rather than through the opcode:
/// (1) a user-defined `Instance`'s own method: `get_attribute` alone returns
/// the raw, UNBOUND `Function` — calling it directly skips `self` entirely,
/// binding whatever the caller's first real argument was to `self` instead
/// (confirmed: `operator.methodcaller('greet', 'world')` on an instance
/// raised `NameError: local variable 'name' referenced before assignment`,
/// because `'world'` silently became `self` and the real `name` parameter
/// was never filled at all).
/// (2) a NATIVE type's method (e.g. `"hello".upper`): these are built with
/// `self_obj: PyObject::None` as a documented PLACEHOLDER meaning "rebind me
/// to whatever object I was actually looked up on" — a rebind step ONLY
/// `LOAD_ATTR`'s own inline copy performs. Skipping it means the returned
/// `BuiltinMethod` keeps `self_obj = None` forever, so calling it later
/// operates on `None` instead of the real object (confirmed:
/// `operator.attrgetter('upper')("hello")()` returned `'NONE'` — the
/// uppercased string representation of `None`, not `"hello"`'s real
/// `.upper()` result `'HELLO'`).
fn bound_attr(obj: &PyObjectRef, name: &str) -> PyResult<PyObjectRef> {
    if matches!(&*obj.borrow(), PyObject::Instance { .. }) {
        if let Ok(Some(bound)) = with_vm_mut(|vm| vm.resolve_descriptor_attr(obj, name)) {
            return Ok(bound);
        }
    }
    let attr = obj.borrow().get_attribute(name)?;
    let needs_rebind = matches!(&*attr.borrow(), PyObject::BuiltinMethod { self_obj, .. } if matches!(&*self_obj.borrow(), PyObject::None));
    if needs_rebind {
        if let PyObject::BuiltinMethod { name: n, func, .. } = &*attr.borrow() {
            return Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                name: n.clone(),
                func: *func,
                self_obj: obj.clone(),
            }));
        }
    }
    Ok(attr)
}

thread_local! {
    // One shared `compare_digest` BuiltinFunction object handed to BOTH
    // `operator._compare_digest` and `hmac.compare_digest`, so CPython's
    // `hmac.compare_digest is _operator._compare_digest` identity check
    // holds (test_hmac.py's `HMACCompareDigestTestCase.test_compare_digest_func`).
    static SHARED_COMPARE_DIGEST: std::cell::RefCell<Option<PyObjectRef>> = std::cell::RefCell::new(None);
}

pub(crate) fn shared_compare_digest() -> PyObjectRef {
    SHARED_COMPARE_DIGEST.with(|c| {
        let mut b = c.borrow_mut();
        if b.is_none() {
            // A Closure (NOT a BuiltinFunction): the LOAD_ATTR opcode's
            // descriptor dispatch auto-binds BuiltinFunctions found on a
            // class into methods that PREPEND `self` — wrong here, since
            // `self.compare_digest(a, b)` (test_hmac.py's pattern, where
            // compare_digest is a plain module function stored as a class
            // attribute) must pass exactly (a, b), not (self, a, b).
            // Closures are deliberately exempt from that auto-binding.
            *b = Some(PyObjectRef::new(PyObject::Closure(std::rc::Rc::new(
                operator_compare_digest_builtin as crate::object::BuiltinFunc,
            ))));
        }
        b.clone().unwrap()
    })
}

pub(crate) fn operator_compare_digest_builtin(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 2 {
        return Err(PyError::type_error("compare_digest requires 2 arguments"));
    }
    let kind = |obj: &PyObjectRef| -> i32 {
        // bytes/str SUBCLASSES (plain `Instance`s wrapping a native backing)
        // behave like their base in real CPython (`PyObject_CheckBuffer` /
        // `PyUnicode_Check` both pass for subclasses) — look through the
        // native backing too.
        let effective = crate::object::native_backing_of(obj).unwrap_or_else(|| obj.clone());
        if matches!(
            &*effective.borrow(),
            PyObject::Bytes(_) | PyObject::ByteArray(_)
        ) {
            return 1;
        }
        let is_small = matches!(&effective, PyObjectRef::SmallStr(_));
        let is_str = if is_small {
            false
        } else {
            matches!(&*effective.borrow(), PyObject::Str(_))
        };
        if is_small || is_str {
            2
        } else {
            0
        }
    };
    let (ka, kb) = (kind(&args[0]), kind(&args[1]));
    let bytes_of = |obj: &PyObjectRef| -> Vec<u8> {
        let effective = crate::object::native_backing_of(obj).unwrap_or_else(|| obj.clone());
        if let PyObjectRef::SmallStr(s) = &effective {
            return s.as_str().as_bytes().to_vec();
        }
        let borrowed = effective.borrow();
        match &*borrowed {
            PyObject::Bytes(b) => b.clone(),
            PyObject::ByteArray(b) => b.clone(),
            PyObject::Str(s) => s.as_bytes().to_vec(),
            _ => vec![],
        }
    };
    if ka == 2 && kb == 2 {
        // str + str: ASCII only (real CPython rejects non-ASCII str).
        let (sa, sb) = (args[0].str(), args[1].str());
        if !sa.is_ascii() || !sb.is_ascii() {
            return Err(PyError::type_error(
                "comparing strings with non-ASCII characters is not supported",
            ));
        }
        return Ok(py_bool(sa.as_bytes() == sb.as_bytes()));
    }
    if ka == 1 && kb == 1 {
        let (a, b) = (bytes_of(&args[0]), bytes_of(&args[1]));
        // Constant-time: a single fold over the max length.
        let mut diff = a.len() ^ b.len();
        for i in 0..a.len().max(b.len()) {
            diff |= (a.get(i).copied().unwrap_or(0) as usize)
                ^ (b.get(i).copied().unwrap_or(0) as usize);
        }
        return Ok(py_bool(diff == 0));
    }
    let ta = args[0].borrow().type_name().to_string();
    let tb = args[1].borrow().type_name().to_string();
    Err(PyError::type_error(format!(
        "unsupported operand types(s) or combination of types: '{}' and '{}'",
        ta, tb
    )))
}

pub fn create_operator_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! op_func {
        ($name:expr, $func:expr) => {
            d.insert(
                $name.to_string(),
                PyObjectRef::new(PyObject::BuiltinFunction {
                    name: $name.to_string(),
                    func: $func,
                }),
            );
        };
    }

    op_func!("add", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("operator.add requires 2 arguments"));
        }
        py_add(&args[0], &args[1])
    });
    op_func!("sub", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("operator.sub requires 2 arguments"));
        }
        py_sub(&args[0], &args[1])
    });
    op_func!("mul", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("operator.mul requires 2 arguments"));
        }
        py_mul(&args[0], &args[1])
    });
    op_func!("truediv", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("operator.truediv requires 2 arguments"));
        }
        py_div(&args[0], &args[1])
    });
    op_func!("floordiv", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error(
                "operator.floordiv requires 2 arguments",
            ));
        }
        py_floor_div(&args[0], &args[1])
    });
    op_func!("mod", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("operator.mod requires 2 arguments"));
        }
        py_mod(&args[0], &args[1])
    });
    op_func!("pow", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("operator.pow requires 2 arguments"));
        }
        py_pow(&args[0], &args[1])
    });
    op_func!("lt", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("operator.lt requires 2 arguments"));
        }
        py_compare(&args[0], &args[1], 0)
    });
    op_func!("le", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("operator.le requires 2 arguments"));
        }
        py_compare(&args[0], &args[1], 1)
    });
    op_func!("eq", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("operator.eq requires 2 arguments"));
        }
        py_compare(&args[0], &args[1], 2)
    });
    op_func!("ne", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("operator.ne requires 2 arguments"));
        }
        py_compare(&args[0], &args[1], 5)
    });
    op_func!("ge", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("operator.ge requires 2 arguments"));
        }
        py_compare(&args[0], &args[1], 3)
    });
    op_func!("gt", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("operator.gt requires 2 arguments"));
        }
        py_compare(&args[0], &args[1], 4)
    });
    op_func!("and_", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("operator.and_ requires 2 arguments"));
        }
        py_bit_and(&args[0], &args[1])
    });
    op_func!("or_", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("operator.or_ requires 2 arguments"));
        }
        py_bit_or(&args[0], &args[1])
    });
    op_func!("xor", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("operator.xor requires 2 arguments"));
        }
        py_bit_xor(&args[0], &args[1])
    });
    op_func!("not_", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("operator.not_ requires 1 argument"));
        }
        py_not(&args[0])
    });
    op_func!("getitem", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("operator.getitem requires 2 arguments"));
        }
        py_getitem(&args[0], &args[1])
    });
    op_func!("setitem", |args| {
        if args.len() < 3 {
            return Err(PyError::type_error("operator.setitem requires 3 arguments"));
        }
        py_setitem(&args[0], &args[1], args[2].clone())?;
        Ok(py_none())
    });
    op_func!("delitem", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("operator.delitem requires 2 arguments"));
        }
        py_delitem(&args[0], &args[1])?;
        Ok(py_none())
    });
    op_func!("contains", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error(
                "operator.contains requires 2 arguments",
            ));
        }
        py_contains(&args[0], &args[1])
    });
    op_func!("index", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("operator.index requires 1 argument"));
        }
        to_index(&args[0]).map(|i| py_int(i))
    });
    op_func!("indexOf", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("operator.indexOf requires 2 arguments"));
        }
        let it = crate::object::builtin_iter(&[args[0].clone()])?;
        let mut idx: i64 = 0;
        loop {
            match crate::object::builtin_next(&[it.clone()]) {
                Ok(v) => {
                    if crate::object::py_compare(&v, &args[1], 2)?.truthy() {
                        return Ok(py_int(idx));
                    }
                    idx += 1;
                }
                Err(PyError::StopIteration) => {
                    return Err(PyError::value_error("sequence.index(x): x not in sequence"))
                }
                Err(e) => return Err(e),
            }
        }
    });
    op_func!("countOf", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("operator.countOf requires 2 arguments"));
        }
        let it = crate::object::builtin_iter(&[args[0].clone()])?;
        let mut count: i64 = 0;
        loop {
            match crate::object::builtin_next(&[it.clone()]) {
                Ok(v) => {
                    if crate::object::py_compare(&v, &args[1], 2)?.truthy() {
                        count += 1;
                    }
                }
                Err(PyError::StopIteration) => return Ok(py_int(count)),
                Err(e) => return Err(e),
            }
        }
    });
    op_func!("truth", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("operator.truth requires 1 argument"));
        }
        Ok(py_bool(args[0].truthy()))
    });
    op_func!("neg", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("operator.neg requires 1 argument"));
        }
        py_neg(&args[0])
    });
    op_func!("pos", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("operator.pos requires 1 argument"));
        }
        Ok(args[0].clone())
    });
    op_func!("abs", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("operator.abs requires 1 argument"));
        }
        if let Some(i) = args[0].as_i64() {
            return Ok(py_int(i.abs()));
        }
        if let Some(f) = args[0].as_f64() {
            return Ok(py_float(f.abs()));
        }
        let obj = args[0].borrow();
        match &*obj {
            PyObject::Int(n) => Ok(py_int(n.clone().abs())),
            PyObject::Float(n) => Ok(py_float(n.abs())),
            _ => Err(PyError::type_error(format!(
                "bad operand type for abs(): '{}'",
                obj.type_name()
            ))),
        }
    });
    op_func!("inv", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("operator.inv requires 1 argument"));
        }
        if let Some(i) = args[0].as_i64() {
            return Ok(py_int(!i));
        }
        let obj = args[0].borrow();
        if let PyObject::Int(n) = &*obj {
            Ok(py_int(!n.clone()))
        } else {
            Err(PyError::type_error(format!(
                "bad operand type for inv(): '{}'",
                obj.type_name()
            )))
        }
    });
    op_func!("lshift", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("operator.lshift requires 2 arguments"));
        }
        py_lshift(&args[0], &args[1])
    });
    op_func!("rshift", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("operator.rshift requires 2 arguments"));
        }
        py_rshift(&args[0], &args[1])
    });
    op_func!("length_hint", |args| {
        if args.is_empty() {
            return Err(PyError::type_error(
                "operator.length_hint requires 1 argument",
            ));
        }
        builtin_len(args)
    });
    // `operator.is_`/`is_not` — plain identity checks, real Python's
    // function-object equivalents of the `is`/`is not` operators (used
    // e.g. as a `key=`/comparison callable where a bare operator won't do).
    // Missing entirely before.
    op_func!("is_", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("operator.is_ requires 2 arguments"));
        }
        Ok(py_bool(args[0].is(&args[1])))
    });
    op_func!("is_not", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("operator.is_not requires 2 arguments"));
        }
        Ok(py_bool(!args[0].is(&args[1])))
    });
    // __iadd__ etc. — just wrap the binop
    op_func!("__add__", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("__add__ requires 2 arguments"));
        }
        py_add(&args[0], &args[1])
    });
    op_func!("__sub__", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("__sub__ requires 2 arguments"));
        }
        py_sub(&args[0], &args[1])
    });
    op_func!("__mul__", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("__mul__ requires 2 arguments"));
        }
        py_mul(&args[0], &args[1])
    });
    op_func!("__truediv__", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("__truediv__ requires 2 arguments"));
        }
        py_div(&args[0], &args[1])
    });
    op_func!("__floordiv__", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("__floordiv__ requires 2 arguments"));
        }
        py_floor_div(&args[0], &args[1])
    });
    op_func!("__mod__", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("__mod__ requires 2 arguments"));
        }
        py_mod(&args[0], &args[1])
    });
    op_func!("__pow__", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("__pow__ requires 2 arguments"));
        }
        py_pow(&args[0], &args[1])
    });
    op_func!("__and__", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("__and__ requires 2 arguments"));
        }
        py_bit_and(&args[0], &args[1])
    });
    op_func!("__or__", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("__or__ requires 2 arguments"));
        }
        py_bit_or(&args[0], &args[1])
    });
    op_func!("__xor__", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("__xor__ requires 2 arguments"));
        }
        py_bit_xor(&args[0], &args[1])
    });
    op_func!("__lshift__", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("__lshift__ requires 2 arguments"));
        }
        py_lshift(&args[0], &args[1])
    });
    op_func!("__rshift__", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("__rshift__ requires 2 arguments"));
        }
        py_rshift(&args[0], &args[1])
    });
    // Aliases for operator.__lt__ etc (test_collections.py's validate_comparison does getattr(operator, '__lt__'))
    op_func!("__lt__", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("__lt__ requires 2 arguments"));
        }
        py_compare(&args[0], &args[1], 0)
    });
    op_func!("__le__", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("__le__ requires 2 arguments"));
        }
        py_compare(&args[0], &args[1], 1)
    });
    op_func!("__eq__", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("__eq__ requires 2 arguments"));
        }
        py_compare(&args[0], &args[1], 2)
    });
    op_func!("__ne__", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("__ne__ requires 2 arguments"));
        }
        py_compare(&args[0], &args[1], 5)
    });
    op_func!("__ge__", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("__ge__ requires 2 arguments"));
        }
        py_compare(&args[0], &args[1], 3)
    });
    op_func!("__gt__", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("__gt__ requires 2 arguments"));
        }
        py_compare(&args[0], &args[1], 4)
    });
    op_func!("__getitem__", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("__getitem__ requires 2 arguments"));
        }
        py_getitem(&args[0], &args[1])
    });
    op_func!("__setitem__", |args| {
        if args.len() < 3 {
            return Err(PyError::type_error("__setitem__ requires 3 arguments"));
        }
        py_setitem(&args[0], &args[1], args[2].clone())?;
        Ok(py_none())
    });

    // itemgetter factory
    d.insert_str(
        "itemgetter",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "itemgetter".to_string(),
            func: |args| {
                if args.is_empty() {
                    return Err(PyError::type_error(
                        "itemgetter requires at least 1 argument",
                    ));
                }
                let items = args.to_vec();
                // Return a callable that does getitem on its argument
                let getter = PyObjectRef::new(PyObject::Closure(Rc::new(move |get_args| {
                    if get_args.is_empty() {
                        return Err(PyError::type_error("itemgetter called with no arguments"));
                    }
                    let obj = &get_args[0];
                    if items.len() == 1 {
                        py_getitem(obj, &items[0])
                    } else {
                        let mut results = Vec::new();
                        for item in &items {
                            results.push(py_getitem(obj, item)?);
                        }
                        Ok(PyObjectRef::imm(PyObject::Tuple(results)))
                    }
                })));
                Ok(getter)
            },
        }),
    );

    // attrgetter factory
    d.insert_str(
        "attrgetter",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "attrgetter".to_string(),
            func: |args| {
                if args.is_empty() {
                    return Err(PyError::type_error(
                        "attrgetter requires at least 1 argument",
                    ));
                }
                let attrs: Vec<String> = args.iter().map(|a| a.str()).collect();
                let getter = PyObjectRef::new(PyObject::Closure(Rc::new(move |get_args| {
                    if get_args.is_empty() {
                        return Err(PyError::type_error("attrgetter called with no arguments"));
                    }
                    if attrs.len() == 1 {
                        bound_attr(&get_args[0], &attrs[0])
                    } else {
                        let mut results = Vec::new();
                        for attr in &attrs {
                            results.push(bound_attr(&get_args[0], attr)?);
                        }
                        Ok(PyObjectRef::imm(PyObject::Tuple(results)))
                    }
                })));
                Ok(getter)
            },
        }),
    );

    // `operator.methodcaller(name, *args)` — missing entirely. Returns a
    // callable that, given `obj`, calls `obj.name(*args)` — a common
    // `key=`/callback idiom (`sorted(objs, key=methodcaller('lower'))`,
    // real trigger: CPython's own `test_operator.py`). Positional args only
    // (no keyword-argument support) — good enough for the common case, and
    // consistent with this module's existing `itemgetter`/`attrgetter`
    // factories just above, neither of which support keywords either.
    d.insert_str(
        "methodcaller",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "methodcaller".to_string(),
            func: |args| {
                if args.is_empty() {
                    return Err(PyError::type_error(
                        "methodcaller requires at least 1 argument",
                    ));
                }
                let method_name = args[0].str();
                let extra_args: Vec<PyObjectRef> = args[1..].to_vec();
                let caller = PyObjectRef::new(PyObject::Closure(Rc::new(move |call_args| {
                    if call_args.is_empty() {
                        return Err(PyError::type_error(
                            "methodcaller's callable requires an argument",
                        ));
                    }
                    let obj = &call_args[0];
                    let method = bound_attr(obj, &method_name)?;
                    let mut full_args = extra_args.clone();
                    full_args.extend_from_slice(&call_args[1..]);
                    builtin_call(&method, &full_args)
                })));
                Ok(caller)
            },
        }),
    );

    // `operator.__all__` — missing entirely (`AttributeError`), breaking
    // even the module's own `test___all__` sanity check at collection time
    // (real trigger: CPython's own `test_operator.py`). Computed from the
    // dict's own already-public (non-dunder) keys rather than a hand-
    // maintained literal list, so it can't drift out of sync with whatever
    // this function actually defines above.
    let all_names: Vec<PyObjectRef> = d
        .keys()
        .filter(|k| !k.starts_with('_'))
        .map(|k| py_str(k))
        .collect();
    d.insert_str("__all__", py_list(all_names));

    // `operator._compare_digest(a, b)` — constant-time bytes comparison
    // (the actual `hmac.compare_digest` primitive; CPython's own
    // `test_hmac.py` imports it directly via `from _operator import
    // _compare_digest` AND asserts `hmac.compare_digest IS
    // _operator._compare_digest` — both dicts must hold the very same
    // Rc object, hence the cached shared instance below). str operands
    // are rejected with the same TypeError CPython raises; anything
    // else gets the generic "unsupported operand types" message.
    d.insert("_compare_digest".to_string(), shared_compare_digest());

    d
}

