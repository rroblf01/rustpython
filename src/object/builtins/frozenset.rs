use super::*;

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
                for item in v {
                    set.add(item.clone())?;
                }
                Ok(PyObjectRef::imm(PyObject::FrozenSet(set)))
            }
            PyObject::Tuple(v) => {
                let mut set = PySet::new();
                for item in v {
                    set.add(item.clone())?;
                }
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
            PyObject::Range { .. } => {
                drop(obj);
                let it = builtin_iter(&[args[0].clone()])?;
                let mut set = PySet::new();
                loop {
                    match builtin_next(&[it.clone()]) {
                        Ok(item) => set.add(item.clone())?,
                        Err(PyError::StopIteration) => break,
                        Err(e) => return Err(e),
                    }
                }
                Ok(PyObjectRef::imm(PyObject::FrozenSet(set)))
            }
            _ => {
                // Fall back to the general iterator protocol instead of a
                // hardcoded list of concrete variants — `frozenset(x)`
                // must accept ANY iterable (a dict view, a generator, a
                // custom class with `__iter__`), not just the specific
                // native shapes special-cased above (which exist only as
                // a fast path). Confirmed real gap via
                // `frozenset(some_dict.keys())` (a dict_keys view is a
                // `PyObject::Instance`, matched neither above nor by the
                // old catch-all) raising "cannot convert 'instance' to
                // frozenset" instead of working — test_dictviews.py's
                // `test_keys_set_operations`/`test_set_operations_with_
                // iterator`. `list()`'s own constructor already does this
                // same iterator fallback; frozenset()/set() didn't.
                drop(obj);
                let it = builtin_iter(&[args[0].clone()]).map_err(|_| {
                    PyError::type_error(format!(
                        "cannot convert '{}' to frozenset",
                        args[0].borrow().type_name()
                    ))
                })?;
                let mut set = PySet::new();
                loop {
                    match builtin_next(&[it.clone()]) {
                        Ok(item) => set.add(item.clone())?,
                        Err(PyError::StopIteration) => break,
                        Err(e) => return Err(e),
                    }
                }
                Ok(PyObjectRef::imm(PyObject::FrozenSet(set)))
            }
        }
    }
}
