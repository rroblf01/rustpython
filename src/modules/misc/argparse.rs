use crate::object::*;
use std::collections::HashMap;

// ─── argparse module ──────────────────────────────────────────────────────────

pub fn create_argparse_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    let mut parser_type_dict = HashMap::new();
    macro_rules! p_method {
        ($name:expr, $func:expr) => {
            parser_type_dict.insert(
                $name.to_string(),
                PyObjectRef::new(PyObject::BuiltinFunction {
                    name: $name.to_string(),
                    func: $func,
                }),
            );
        };
    }

    p_method!("__init__", |_args| {
        // Accept optional description (first arg after self)
        // self is args[0], description would be args[1]
        Ok(py_none())
    });

    p_method!("add_argument", |_args| {
        // Stub: return None
        Ok(py_none())
    });

    p_method!("parse_args", |args| {
        // Create Namespace instance
        let ns_type = PyObjectRef::new(PyObject::Type {
            name: "Namespace".to_string(),
            dict: Box::new(str_map_to_typedict(HashMap::new())),
            bases: vec![],
            mro: vec![],
        });

        let mut ns_dict = AttrMap::new();
        if args.len() > 1 {
            let arg_list: Vec<String> = {
                let borrowed = args[1].borrow();
                if let PyObject::List(list) = &*borrowed {
                    list.iter().map(|s| s.str()).collect()
                } else {
                    return Err(PyError::type_error(
                        "parse_args: expected a list of strings",
                    ));
                }
            };
            let mut i = 0;
            while i < arg_list.len() {
                let a = &arg_list[i];
                if a.starts_with("--") {
                    let name = a.trim_start_matches('-');
                    let (key, val) = if let Some(eq_pos) = name.find('=') {
                        (name[..eq_pos].to_string(), py_str(&name[eq_pos + 1..]))
                    } else {
                        i += 1;
                        if i < arg_list.len() && !arg_list[i].starts_with('-') {
                            (name.to_string(), py_str(&arg_list[i]))
                        } else {
                            (name.to_string(), py_bool(true))
                        }
                    };
                    ns_dict.insert(key.replace('-', "_"), val);
                } else if a.starts_with('-') && a.len() == 2 {
                    let flag = a[1..].to_string();
                    i += 1;
                    if i < arg_list.len() && !arg_list[i].starts_with('-') {
                        ns_dict.insert(flag, py_str(&arg_list[i]));
                    } else {
                        ns_dict.insert(flag, py_bool(true));
                    }
                }
                i += 1;
            }
        }

        Ok(PyObjectRef::new(PyObject::Instance {
            typ: ns_type,
            dict: ns_dict,
        }))
    });

    let parser_type = PyObjectRef::new(PyObject::Type {
        name: "ArgumentParser".to_string(),
        dict: Box::new(str_map_to_typedict(parser_type_dict)),
        bases: vec![],
        mro: vec![],
    });

    d.insert_str("ArgumentParser", parser_type);
    // Action subclasses needed by Django management commands
    fn make_action(name: &str) -> PyObjectRef {
        PyObjectRef::new(PyObject::Type {
            name: name.to_string(),
            dict: Box::new(str_map_to_typedict(HashMap::new())),
            bases: vec![],
            mro: vec![],
        })
    }
    d.insert_str("HelpFormatter", make_action("HelpFormatter"));
    d.insert_str("SUPPRESS", py_str("==SUPPRESS=="));
    d.insert_str("_AppendConstAction", make_action("_AppendConstAction"));
    d.insert_str("_CountAction", make_action("_CountAction"));
    d.insert_str("_StoreConstAction", make_action("_StoreConstAction"));
    d.insert_str("_SubParsersAction", make_action("_SubParsersAction"));
    d
}
