use crate::object::*;
use std::collections::HashMap;

pub fn create_sysconfig_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! syscfg_func {
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

    syscfg_func!("get_config_var", |args| {
        if args.is_empty() {
            return Err(PyError::type_error(
                "get_config_var() missing required argument (name)",
            ));
        }
        Ok(py_none())
    });

    syscfg_func!("get_config_vars", |_| { Ok(py_dict()) });

    syscfg_func!("get_platform", |_| { Ok(py_str("linux-x86_64")) });

    // sysconfig.get_path(name, ...) — returns install paths; pydoc reads
    // get_path('stdlib') to locate module docstrings. Return the interpreter's
    // Lib dir (sys.path[0] resolved through the live sys module).
    syscfg_func!("get_path", |args| {
        let name = args.first().map(|a| a.str()).unwrap_or_default();
        let base = crate::modules::get_module("sys").and_then(|m| {
            let b = m.borrow();
            if let PyObject::Module { dict, .. } = &*b {
                dict.get_str("path").cloned()
            } else {
                None
            }
        });
        if let Some(path_list) = base {
            let p = {
                let pb = path_list.borrow();
                if let PyObject::List(items) = &*pb {
                    items.first().map(|i| i.str())
                } else {
                    None
                }
            };
            if let Some(p) = p {
                if !p.is_empty() {
                    let r = match name.as_str() {
                        "stdlib" => format!("{}/Lib", p),
                        "platstdlib" => format!("{}/Lib", p),
                        "purelib" | "platlib" | "include" | "platinclude" | "scripts" | "data" => {
                            p.clone()
                        }
                        _ => p.clone(),
                    };
                    return Ok(py_str(&r));
                }
            }
        }
        Ok(py_str(""))
    });

    syscfg_func!("get_python_version", |_| { Ok(py_str("3.13")) });
    syscfg_func!("_get_python_version_abi", |_| { Ok(py_str("3.13")) });

    syscfg_func!("is_python_build", |_| { Ok(py_bool(false)) });

    d
}
