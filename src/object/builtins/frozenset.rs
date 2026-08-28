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
            _ => Err(PyError::type_error(format!(
                "cannot convert '{}' to frozenset",
                obj.type_name()
            ))),
        }
    }
}
