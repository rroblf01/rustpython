use crate::object::*;
use std::collections::HashMap;

pub fn create_tarfile_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! tar_func {
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

    tar_func!("open", |args| {
        if args.len() < 1 {
            return Err(PyError::type_error(
                "tarfile.open() takes at least 1 argument (name)",
            ));
        }
        let _name = args[0].borrow().str();
        // Return an Instance with getnames() and extractall() methods
        let mut inst_dict = AttrMap::new();
        inst_dict.insert_str("name", py_str(&_name));
        inst_dict.insert_str(
            "getnames",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "getnames".to_string(),
                func: |_args| Ok(py_list(vec![])),
            }),
        );
        inst_dict.insert_str(
            "extractall",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "extractall".to_string(),
                func: |_args| Ok(py_none()),
            }),
        );
        Ok(PyObjectRef::new(PyObject::Instance {
            typ: PyObjectRef::new(PyObject::Module {
                name: "tarfile.TarFile".to_string(),
                dict: Box::new(str_map_to_typedict(HashMap::new())),
            }),
            dict: inst_dict,
        }))
    });

    // tarfile.TarFile type — real CPython exposes TarFile as a class so
    // `from tarfile import TarFile` / `isinstance(t, tarfile.TarFile)` work.
    let mut tarfile_type_dict = HashMap::new();
    tarfile_type_dict.insert(
        "__init__".to_string(),
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__init__".to_string(),
            func: |_args| Ok(py_none()),
        }),
    );
    let tar_file_type = PyObjectRef::new(PyObject::Type {
        name: "TarFile".to_string(),
        dict: Box::new(str_map_to_typedict(tarfile_type_dict)),
        bases: vec![],
        mro: vec![],
    });
    d.insert_str("TarFile", tar_file_type);

    d
}
