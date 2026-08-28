// Auto-extracted from src/object/attrs/mod.rs lines 794-826
use crate::object::*;
use super::*;
use crate::interner;

pub(crate) fn get(o: &PyObject, name: &str) -> PyResult<PyObjectRef> {
    match o {
            PyObject::Module {
                dict,
                name: mod_name,
            } => {
                if name == "__dict__" {
                    // Convert module's HashMap to a PyDict

                    let mut pd = PyDict::new();
                    for (k, v) in dict.iter() {
                        let _ = pd.set(py_str(interner::lookup_str(*k)), v.clone());
                    }
                    return Ok(PyObjectRef::new(PyObject::Dict(Box::new(pd))));
                }
                if name == "__name__" {
                    return Ok(py_str(mod_name));
                }
                dict.get_str(&name).cloned().ok_or_else(|| {
                    if std::env::var("RPY_DEBUG_ATTR").is_ok() {
                        eprintln!(
                            "MODULE_ATTR_FAIL: module={} attr={} keys={:?}",
                            mod_name,
                            name,
                            {
                                let mut ks: Vec<&str> =
                                    dict.keys().map(|k| interner::lookup_str(*k)).collect();
                                ks.sort();
                                ks
                            }
                        );
                    }
                    PyError::attribute_error(format!("'module' object has no attribute '{}'", name))
                })
            }
        _ => unreachable!("{} called with non-{} object", stringify!(get), o.type_name()),
    }
}
