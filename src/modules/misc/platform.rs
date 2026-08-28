use crate::object::*;
use std::collections::HashMap;

pub fn create_platform_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! plat_func {
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
    plat_func!("platform", |_| {
        let os = std::env::consts::OS;
        let arch = std::env::consts::ARCH;
        Ok(py_str(&format!("{}-{}", os, arch)))
    });
    plat_func!("machine", |_| { Ok(py_str(std::env::consts::ARCH)) });
    plat_func!("processor", |_| {
        // Fall back to architecture string if no more specific info
        Ok(py_str(std::env::consts::ARCH))
    });
    plat_func!("python_implementation", |_| { Ok(py_str("RustPython")) });
    plat_func!("python_version", |_| { Ok(py_str("3.12.0")) });
    plat_func!("system", |_| { Ok(py_str(std::env::consts::OS)) });
    plat_func!("release", |_| { Ok(py_str("")) });
    // Real signature: libc_ver(executable=None, lib='', version='',
    // chunksize=16384) -> (lib, version) — detects glibc/musl via parsing
    // the executable's dynamic-linker strings on real CPython. Honest
    // empty-string stub (matches what real CPython itself reports for a
    // non-Linux or otherwise-undetectable target) rather than guessing.
    plat_func!("libc_ver", |_| {
        Ok(py_tuple(vec![py_str(""), py_str("")]))
    });
    // Windows-only in real CPython (returns e.g. "ServerStandard" on
    // Windows Server); always "" elsewhere, which is what non-Windows
    // `platform.py` itself falls back to.
    plat_func!("win32_edition", |_| { Ok(py_str("")) });
    // `platform.uname()` — was missing entirely. Real CPython returns a
    // structseq (`uname_result`) with 6 named fields (`system`, `node`,
    // `release`, `version`, `machine`, `processor`) that's ALSO index/
    // iterable like a plain tuple. Built the same way as `time.
    // struct_time` (a synthetic cached `Type` + `Instance`, see
    // `modules/time.rs`) rather than a plain tuple, since `.system`/
    // `.machine`-style attribute access is the far more common real-world
    // usage pattern.
    plat_func!("uname", |_| {
        let mut dict = crate::object::AttrMap::new();
        let system = py_str(std::env::consts::OS);
        let node = py_str(&std::env::var("HOSTNAME").unwrap_or_default());
        let machine = py_str(std::env::consts::ARCH);
        dict.insert_str("system", system.clone());
        dict.insert_str("node", node.clone());
        dict.insert_str("release", py_str(""));
        dict.insert_str("version", py_str(""));
        dict.insert_str("machine", machine.clone());
        dict.insert_str("processor", py_str(std::env::consts::ARCH));
        Ok(PyObjectRef::new(PyObject::Instance {
            typ: get_uname_result_type(),
            dict,
        }))
    });
    d
}

thread_local! {
    static UNAME_RESULT_TYPE: std::cell::RefCell<Option<PyObjectRef>> = std::cell::RefCell::new(None);
}

const UNAME_FIELDS: [&str; 6] = [
    "system",
    "node",
    "release",
    "version",
    "machine",
    "processor",
];

fn build_uname_result_type() -> PyObjectRef {
    let mut type_dict: HashMap<String, PyObjectRef> = HashMap::new();
    macro_rules! bf {
        ($name:expr, $f:expr) => {
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: $name.to_string(),
                func: $f,
            })
        };
    }
    type_dict.insert_str(
        "__getitem__",
        bf!("__getitem__", |args| {
            if args.len() < 2 {
                return Err(PyError::type_error(
                    "__getitem__() takes exactly one argument",
                ));
            }
            let idx = args[1]
                .as_i64()
                .ok_or_else(|| PyError::type_error("indices must be integers"))?;
            if let PyObject::Instance { dict, .. } = &*args[0].borrow() {
                let i = if idx < 0 { idx + 6 } else { idx };
                let name = UNAME_FIELDS
                    .get(i as usize)
                    .ok_or_else(|| PyError::index_error("uname_result index out of range"))?;
                Ok(dict.get(name).cloned().unwrap_or_else(py_none))
            } else {
                Err(PyError::runtime_error("__getitem__ on non-uname_result"))
            }
        }),
    );
    type_dict.insert_str("__len__", bf!("__len__", |_| Ok(py_int(6))));
    type_dict.insert_str(
        "__iter__",
        bf!("__iter__", |args| {
            if let PyObject::Instance { dict, .. } = &*args[0].borrow() {
                let items: Vec<PyObjectRef> = UNAME_FIELDS
                    .iter()
                    .map(|f| dict.get(f).cloned().unwrap_or_else(py_none))
                    .collect();
                Ok(PyObjectRef::new(PyObject::ListIter {
                    list: items,
                    index: 0,
                }))
            } else {
                Err(PyError::runtime_error("__iter__ on non-uname_result"))
            }
        }),
    );
    type_dict.insert_str(
        "__repr__",
        bf!("__repr__", |args| {
            if let PyObject::Instance { dict, .. } = &*args[0].borrow() {
                let body = UNAME_FIELDS
                    .iter()
                    .map(|f| {
                        format!(
                            "{}={}",
                            f,
                            dict.get(f)
                                .map(|v| v.repr())
                                .unwrap_or_else(|| "None".to_string())
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                Ok(py_str(&format!("uname_result({})", body)))
            } else {
                Ok(py_str("uname_result(...)"))
            }
        }),
    );
    PyObjectRef::new(PyObject::Type {
        name: "platform.uname_result".to_string(),
        dict: Box::new(str_map_to_typedict(type_dict)),
        bases: vec![],
        mro: vec![],
    })
}

fn get_uname_result_type() -> PyObjectRef {
    let existing = UNAME_RESULT_TYPE.with(|c| c.borrow().clone());
    if let Some(t) = existing {
        return t;
    }
    let typ = build_uname_result_type();
    UNAME_RESULT_TYPE.with(|c| {
        *c.borrow_mut() = Some(typ.clone());
    });
    typ
}
