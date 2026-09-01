// Auto-extracted from src/object/attrs/mod.rs lines 1851-1898
use crate::object::*;
use super::*;
use crate::interner;

pub(crate) fn get(o: &PyObject, name: &str) -> PyResult<PyObjectRef> {
    match o {
            PyObject::ExceptionGroup {
                typ,
                args,
                exceptions,
            } => match name {
                "__name__" => Ok(py_str(typ)),
                "args" => Ok(py_tuple(args.clone())),
                "__str__" => {
                    let parts: Vec<String> = args.iter().map(|a| a.str()).collect();
                    Ok(py_str(&parts.join(", ")))
                }
                "__repr__" => {
                    let parts: Vec<String> = args.iter().map(|a| a.repr()).collect();
                    Ok(py_str(&format!("{}({})", typ, parts.join(", "))))
                }
                "message" => Ok(args.first().cloned().unwrap_or_else(|| py_str(""))),
                "exceptions" => Ok(py_tuple(exceptions.clone())),
                "__cause__" | "__context__" | "__traceback__" => Ok(py_none()),
                "__suppress_context__" => Ok(py_bool(false)),
                "__notes__" => Ok(py_list(vec![])),
                "add_note" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "add_note".to_string(),
                    func: |_args| Ok(py_none()),
                    self_obj: PyObjectRef::new(PyObject::None),
                })),
                "with_traceback" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "with_traceback".to_string(),
                    func: |args| {
                        if args.len() < 2 {
                            return Err(PyError::type_error(
                                "with_traceback() takes exactly one argument",
                            ));
                        }
                        // Store the traceback so `raise X().with_traceback(tb)`
                        // yields `X.__traceback__` chaining tb (the RAISE
                        // unwind prepends the current frame's own node).
                        args[0]
                            .borrow_mut()
                            .set_attribute("__traceback__", args[1].clone())?;
                        Ok(args[0].clone())
                    },
                    self_obj: PyObjectRef::new(PyObject::None),
                })),
                _ => Err(PyError::attribute_error(format!(
                    "'{}' object has no attribute '{}'",
                    typ, name
                ))),
            },
        _ => unreachable!("{} called with non-{} object", stringify!(get), o.type_name()),
    }
}
