use crate::object::*;
use num_traits::{Signed, ToPrimitive};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::AtomicI64;

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
        // CPython's precedence: the INSTANCE dict wins for a plain
        // (non-data-descriptor) value, descriptors on the type second.
        // Consulting `resolve_descriptor_attr` FIRST handed back the CLASS
        // attribute, so `attrgetter('y')` on a dataclass instance whose
        // field also has a class-level default returned the default instead
        // of the instance's value (`copy.replace(C(11, 22)).y` → 0, and
        // test_copy.py's `TestReplace.test_dataclass` failed with
        // `(11, 0) != (11, 22)`).
        let instance_value = match &*obj.borrow() {
            PyObject::Instance { dict, .. } => dict.get_str(name).cloned(),
            _ => None,
        };
        if let Some(value) = instance_value {
            // A DATA descriptor (one with `__set__`/`__delete__`, i.e. a
            // `property`) still wins over the instance dict; anything else
            // found on the type (a plain class attribute, a function, …)
            // does not, which is exactly why the instance value is
            // returned here.
            let resolved = with_vm_mut(|vm| vm.resolve_descriptor_attr(obj, name))
                .ok()
                .flatten();
            let is_data_descriptor = match resolved {
                Some(found) => {
                    matches!(found.borrow().get_attribute("__set__"), Ok(_))
                        || matches!(found.borrow().get_attribute("__delete__"), Ok(_))
                }
                None => false,
            };
            if !is_data_descriptor {
                return Ok(value);
            }
        }
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
        if args.len() != 2 {
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
        Ok(py_bool(args[0].try_truthy()?))
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
        py_pos(&args[0])
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
        let default = args.get(1).cloned().unwrap_or_else(|| py_int(0));
        // Real semantics (CPython's `operator.length_hint`): try `len(obj)`
        // first; if that fails with TypeError (no `__len__`), fall back to
        // `obj.__length_hint__()`; if THAT is also missing/fails, return
        // `default`. Previously this just called `len()` directly, so any
        // object with only `__length_hint__` (e.g. `dict_keyiterator`,
        // `deque_reverse_iterator` — real iterators, no public `__len__`)
        // raised TypeError instead of returning the actual hint
        // (test_iterlen.py).
        if let Ok(v) = builtin_len(std::slice::from_ref(&args[0])) {
            return Ok(v);
        }
        match bound_attr(&args[0], "__length_hint__") {
            Ok(method) => match builtin_call(&method, &[]) {
                Ok(v) => {
                    if crate::object::is_not_implemented(&v) {
                        Ok(default)
                    } else {
                        Ok(v)
                    }
                }
                Err(_) => Ok(default),
            },
            Err(_) => Ok(default),
        }
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

    // `itemgetter`/`attrgetter`/`methodcaller` used to be built as bare
    // `PyObject::Closure`s capturing their construction args in a Rust
    // closure — functionally correct for calling, but a `Closure` has no
    // attribute storage at all, so it could never support `repr()` (real
    // CPython: `operator.itemgetter(1, 2)` reprs as
    // `operator.itemgetter(1, 2)`) or `pickle` (real CPython implements
    // `__reduce__` returning `(type(self), self._items)`, since a bound
    // Rust/C closure is inherently unpicklable but the (type, args) it was
    // built from is). Every instance of these factories was reported as
    // `TypeError: cannot pickle builtin_function_or_method object` by
    // `test_operator.py`'s pickle round-trip tests. Fixed generally by
    // making each a real `PyObject::Type`-backed instance (same pattern as
    // `OrderedDict`/`deque` above) storing its constructor args under a
    // reserved instance-dict key, with `__call__`/`__repr__`/`__reduce__`/
    // `__eq__` implemented against that stored state instead of a captured
    // closure.
    const OG_ITEMS_KEY: &str = "__og_items__";
    // `methodcaller`-only: the packed keyword-arguments dict (empty dict
    // when none were given), stored separately from `OG_ITEMS_KEY` (which
    // holds `(name, *args)`) so `methodcaller_call` can forward it and
    // `methodcaller_repr`/`__reduce__` can render/round-trip it.
    const OG_KWARGS_KEY: &str = "__og_kwargs__";

    fn make_getter_type(
        name: &str,
        items: Vec<PyObjectRef>,
        call: BuiltinFunc,
        repr: BuiltinFunc,
    ) -> PyObjectRef {
        let mut type_dict: HashMap<String, PyObjectRef> = HashMap::new();
        type_dict.insert_str(
            "__call__",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "__call__".to_string(),
                func: call,
            }),
        );
        type_dict.insert_str(
            "__repr__",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "__repr__".to_string(),
                func: repr,
            }),
        );
        type_dict.insert_str(
            "__reduce__",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "__reduce__".to_string(),
                func: |args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                    let stored_items = args[0]
                        .borrow()
                        .get_attribute(OG_ITEMS_KEY)?;
                    let typ = if let PyObject::Instance { typ, .. } = &*args[0].borrow() {
                        typ.clone()
                    } else {
                        return Err(PyError::type_error("__reduce__ on non-instance"));
                    };
                    // `methodcaller` only: re-append its stored kwargs dict
                    // as one extra trailing item so reconstruction
                    // (`typ(*stored_items)`) round-trips through the same
                    // trailing-packed-kwargs-dict detection the factory
                    // closure above already does — `itemgetter`/
                    // `attrgetter` never set this key, so they fall
                    // through unchanged.
                    let combined_items = match args[0].borrow().get_attribute(OG_KWARGS_KEY) {
                        Ok(kwargs)
                            if !matches!(&*kwargs.borrow(), PyObject::Dict(d) if d.is_empty()) =>
                        {
                            let mut items = match &*stored_items.borrow() {
                                PyObject::Tuple(t) => t.clone(),
                                _ => Vec::new(),
                            };
                            items.push(kwargs);
                            py_tuple(items)
                        }
                        _ => stored_items,
                    };
                    Ok(py_tuple(vec![typ, combined_items]))
                },
            }),
        );
        type_dict.insert_str(
            "__eq__",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "__eq__".to_string(),
                func: |args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                    if args.len() < 2 {
                        return Ok(py_bool(false));
                    }
                    let a_items = args[0].borrow().get_attribute(OG_ITEMS_KEY)?;
                    let b_items = match args[1].borrow().get_attribute(OG_ITEMS_KEY) {
                        Ok(v) => v,
                        Err(_) => return Ok(py_not_implemented()),
                    };
                    // Also require same type (an itemgetter must not equal
                    // an attrgetter with identical args).
                    let same_type = match (&*args[0].borrow(), &*args[1].borrow()) {
                        (
                            PyObject::Instance { typ: t1, .. },
                            PyObject::Instance { typ: t2, .. },
                        ) => container_of(t1) == container_of(t2),
                        _ => false,
                    };
                    fn container_of(t: &PyObjectRef) -> usize {
                        t.get_id()
                    }
                    if !same_type {
                        return Ok(py_bool(false));
                    }
                    Ok(py_bool(a_items.equals(&b_items)?))
                },
            }),
        );
        let typ = PyObjectRef::new(PyObject::Type {
            name: name.to_string(),
            dict: Box::new(str_map_to_typedict(type_dict)),
            bases: vec![],
            mro: vec![],
        });
        if let PyObject::Type { dict, .. } = &mut *typ.borrow_mut() {
            dict.insert_str("__module__", py_str("operator"));
        }
        if let PyObject::Type { mro, .. } = &mut *typ.borrow_mut() {
            *mro = vec![typ.clone()];
        }
        let mut attrs = crate::object::AttrMap::new();
        attrs.insert(OG_ITEMS_KEY.to_string(), py_tuple(items));
        // Return the TYPE — the caller wraps construction below (needed
        // since each call to the factory produces one fresh Instance).
        let _ = attrs; // constructed per-call below; this fn only builds the type.
        typ
    }

    // itemgetter factory
    thread_local! {
        static ITEMGETTER_TYPE: std::cell::RefCell<Option<PyObjectRef>> = std::cell::RefCell::new(None);
        static ATTRGETTER_TYPE: std::cell::RefCell<Option<PyObjectRef>> = std::cell::RefCell::new(None);
        static METHODCALLER_TYPE: std::cell::RefCell<Option<PyObjectRef>> = std::cell::RefCell::new(None);
    }

    fn itemgetter_call(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
        if args.len() < 2 {
            return Err(PyError::type_error("itemgetter called with no arguments"));
        }
        let items = match &*args[0].borrow().get_attribute(OG_ITEMS_KEY)?.borrow() {
            PyObject::Tuple(t) => t.clone(),
            _ => Vec::new(),
        };
        let obj = &args[1];
        if items.len() == 1 {
            py_getitem(obj, &items[0])
        } else {
            let mut results = Vec::new();
            for item in &items {
                results.push(py_getitem(obj, item)?);
            }
            Ok(PyObjectRef::imm(PyObject::Tuple(results)))
        }
    }
    fn itemgetter_repr(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
        let items = match &*args[0].borrow().get_attribute(OG_ITEMS_KEY)?.borrow() {
            PyObject::Tuple(t) => t.clone(),
            _ => Vec::new(),
        };
        let parts: Vec<String> = items.iter().map(|i| i.repr()).collect();
        Ok(py_str(&format!(
            "operator.itemgetter({})",
            parts.join(", ")
        )))
    }
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
                let typ = ITEMGETTER_TYPE.with(|c| {
                    let mut b = c.borrow_mut();
                    if b.is_none() {
                        *b = Some(make_getter_type(
                            "itemgetter",
                            vec![],
                            itemgetter_call as BuiltinFunc,
                            itemgetter_repr as BuiltinFunc,
                        ));
                    }
                    b.clone().unwrap()
                });
                let mut attrs = crate::object::AttrMap::new();
                attrs.insert(OG_ITEMS_KEY.to_string(), py_tuple(args.to_vec()));
                Ok(PyObjectRef::new(PyObject::Instance { typ, dict: attrs }))
            },
        }),
    );

    // attrgetter factory
    // Real `operator.attrgetter('a.b.c')` resolves a DOTTED path — each `.`
    // segment is a further `getattr` on the previous result — not a single
    // literal attribute named `"a.b.c"`. `bound_attr` (above) does a single
    // lookup, so it must be driven once per dotted segment here.
    fn attrgetter_one(obj: &PyObjectRef, path: &str) -> PyResult<PyObjectRef> {
        let mut cur = obj.clone();
        for segment in path.split('.') {
            cur = bound_attr(&cur, segment)?;
        }
        Ok(cur)
    }
    fn attrgetter_call(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
        if args.len() < 2 {
            return Err(PyError::type_error("attrgetter called with no arguments"));
        }
        let attrs_tuple = match &*args[0].borrow().get_attribute(OG_ITEMS_KEY)?.borrow() {
            PyObject::Tuple(t) => t.clone(),
            _ => Vec::new(),
        };
        let attrs: Vec<String> = attrs_tuple.iter().map(|a| a.str()).collect();
        let obj = &args[1];
        if attrs.len() == 1 {
            attrgetter_one(obj, &attrs[0])
        } else {
            let mut results = Vec::new();
            for attr in &attrs {
                results.push(attrgetter_one(obj, attr)?);
            }
            Ok(PyObjectRef::imm(PyObject::Tuple(results)))
        }
    }
    fn attrgetter_repr(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
        let items = match &*args[0].borrow().get_attribute(OG_ITEMS_KEY)?.borrow() {
            PyObject::Tuple(t) => t.clone(),
            _ => Vec::new(),
        };
        let parts: Vec<String> = items.iter().map(|i| i.repr()).collect();
        Ok(py_str(&format!(
            "operator.attrgetter({})",
            parts.join(", ")
        )))
    }
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
                let typ = ATTRGETTER_TYPE.with(|c| {
                    let mut b = c.borrow_mut();
                    if b.is_none() {
                        *b = Some(make_getter_type(
                            "attrgetter",
                            vec![],
                            attrgetter_call as BuiltinFunc,
                            attrgetter_repr as BuiltinFunc,
                        ));
                    }
                    b.clone().unwrap()
                });
                let mut attrs = crate::object::AttrMap::new();
                attrs.insert(OG_ITEMS_KEY.to_string(), py_tuple(args.to_vec()));
                Ok(PyObjectRef::new(PyObject::Instance { typ, dict: attrs }))
            },
        }),
    );

    // `operator.methodcaller(name, *args, **kwargs)` — a callable that,
    // given `obj`, calls `obj.name(*args, **kwargs)` — a common
    // `key=`/callback idiom (`sorted(objs, key=methodcaller('lower'))`).
    // Keyword arguments to the wrapped call are NOT supported (matching the
    // pre-existing behavior — positional-only), but the stored args now
    // round-trip through pickle/repr/equality like real CPython's.
    fn methodcaller_call(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
        if args.len() < 2 {
            return Err(PyError::type_error(
                "methodcaller's callable requires an argument",
            ));
        }
        let items = match &*args[0].borrow().get_attribute(OG_ITEMS_KEY)?.borrow() {
            PyObject::Tuple(t) => t.clone(),
            _ => Vec::new(),
        };
        if items.is_empty() {
            return Err(PyError::type_error(
                "methodcaller requires at least 1 argument",
            ));
        }
        let method_name = items[0].str();
        let extra_args = &items[1..];
        let obj = &args[1];
        let method = bound_attr(obj, &method_name)?;
        let mut full_args = extra_args.to_vec();
        full_args.extend_from_slice(&args[2..]);
        // Stored keyword arguments (`operator.methodcaller(name, **kwds)`)
        // are forwarded as this project's usual trailing packed-kwargs-dict
        // ARGUMENT — same convention `builtin_call`'s callees already
        // expect — rather than being silently dropped (the previous
        // behavior, per this function's now-stale "positional-only"
        // comment above), which broke every keyword-arg case in
        // `test_operator.py::test_methodcaller`.
        if let Ok(kwargs) = args[0].borrow().get_attribute(OG_KWARGS_KEY) {
            if !matches!(&*kwargs.borrow(), PyObject::Dict(d) if d.is_empty()) {
                full_args.push(kwargs);
            }
        }
        builtin_call(&method, &full_args)
    }
    fn methodcaller_repr(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
        let items = match &*args[0].borrow().get_attribute(OG_ITEMS_KEY)?.borrow() {
            PyObject::Tuple(t) => t.clone(),
            _ => Vec::new(),
        };
        if items.is_empty() {
            return Ok(py_str("operator.methodcaller()"));
        }
        let name_repr = items[0].str();
        let rest: Vec<String> = items[1..].iter().map(|i| i.repr()).collect();
        if rest.is_empty() {
            Ok(py_str(&format!("operator.methodcaller('{}')", name_repr)))
        } else {
            Ok(py_str(&format!(
                "operator.methodcaller('{}', {})",
                name_repr,
                rest.join(", ")
            )))
        }
    }
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
                if !matches!(&*args[0].borrow(), PyObject::Str(_)) {
                    return Err(PyError::type_error("method name must be a string"));
                }
                let typ = METHODCALLER_TYPE.with(|c| {
                    let mut b = c.borrow_mut();
                    if b.is_none() {
                        *b = Some(make_getter_type(
                            "methodcaller",
                            vec![],
                            methodcaller_call as BuiltinFunc,
                            methodcaller_repr as BuiltinFunc,
                        ));
                    }
                    b.clone().unwrap()
                });
                // Detect this project's usual trailing packed-kwargs dict
                // (present when `methodcaller(name, **kwds)` was used) and
                // store it separately from the positional `(name, *args)`
                // tuple, instead of folding it into `OG_ITEMS_KEY` where
                // `methodcaller_call` used to just silently ignore it.
                let (positional, kwargs) = if args.len() >= 2
                    && matches!(&*args[args.len() - 1].borrow(), PyObject::Dict(_))
                {
                    (&args[..args.len() - 1], args[args.len() - 1].clone())
                } else {
                    (
                        args,
                        PyObjectRef::new(PyObject::Dict(Box::new(PyDict::new()))),
                    )
                };
                let mut attrs = crate::object::AttrMap::new();
                attrs.insert(OG_ITEMS_KEY.to_string(), py_tuple(positional.to_vec()));
                attrs.insert(OG_KWARGS_KEY.to_string(), kwargs);
                Ok(PyObjectRef::new(PyObject::Instance { typ, dict: attrs }))
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
