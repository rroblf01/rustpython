use crate::object::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

// ============================================================
// contextvars module — ContextVar with thread-local storage
// ============================================================

thread_local! {
    /// Per-variable history stacks: name -> Vec<(token_id, value)>
    static CONTEXT_DATA: RefCell<HashMap<String, Vec<(u64, PyObjectRef)>>> = RefCell::new(HashMap::new());
    /// Auto-incrementing token counter
    static NEXT_TOKEN: RefCell<u64> = RefCell::new(1);
}

/// Helper to get the current value of a ContextVar by name, or None if not set
fn context_var_get_value(name: &str) -> Option<PyObjectRef> {
    CONTEXT_DATA.with(|cell| {
        let map = cell.borrow();
        map.get(name)
            .and_then(|stack| stack.last().map(|(_, v)| v.clone()))
    })
}

pub fn create_contextvars_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();

    // ---- ContextVar type ----
    let mut contextvar_type_dict: HashMap<String, PyObjectRef> = HashMap::new();

    macro_rules! cv_method {
        ($name:expr, $func:expr) => {
            contextvar_type_dict.insert(
                $name.to_string(),
                PyObjectRef::new(PyObject::BuiltinFunction {
                    name: $name.to_string(),
                    func: $func,
                }),
            );
        };
    }

    // __init__(self, name, default=None)
    cv_method!("__init__", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error(
                "ContextVar() requires at least 1 argument (name)",
            ));
        }
        let name = args[1].str();
        if let PyObject::Instance { dict, .. } = &mut *args[0].borrow_mut() {
            dict.insert_str("_name", py_str(&name));
            let default = if args.len() > 2 {
                args[2].clone()
            } else {
                py_none()
            };
            dict.insert_str("_default", default);
        }
        Ok(py_none())
    });

    // name property getter
    {
        let getter = PyObjectRef::new(PyObject::BuiltinFunction {
            name: "name".to_string(),
            func: |args| {
                if args.is_empty() {
                    return Err(PyError::type_error("name getter missing argument"));
                }
                let instance = &args[0];
                let borrowed = instance.borrow();
                if let PyObject::Instance { dict, .. } = &*borrowed {
                    if let Some(name_val) = dict.get_str("_name") {
                        return Ok(name_val.clone());
                    }
                }
                Err(PyError::type_error("ContextVar instance has no _name"))
            },
        });
        contextvar_type_dict.insert_str(
            "name",
            PyObjectRef::new(PyObject::Property(Box::new(PropertyData {
                getter: Some(getter),
                setter: None,
                deleter: None,
                doc: None,
            }))),
        );
    }

    // get(self, default=None)
    cv_method!("get", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("get() missing self argument"));
        }
        let instance = &args[0];

        // Extract name and default from the instance
        let (name, default) = {
            let borrowed = instance.borrow();
            if let PyObject::Instance { dict, .. } = &*borrowed {
                let nm = dict
                    .get_str("_name")
                    .ok_or_else(|| PyError::type_error("ContextVar instance has no _name"))?
                    .str();
                let df = dict.get_str("_default").cloned().unwrap_or(py_none());
                (nm, df)
            } else {
                return Err(PyError::type_error("not a ContextVar instance"));
            }
        };

        // Look up current value in thread-local storage
        match context_var_get_value(&name) {
            Some(val) => Ok(val),
            None => {
                // Use default passed as argument, or the ContextVar's default
                if args.len() > 1 {
                    Ok(args[1].clone())
                } else if matches!(default, PyObjectRef::None) {
                    Err(PyError::key_error(format!(
                        "ContextVar '{}' has no value and no default",
                        name
                    )))
                } else {
                    Ok(default)
                }
            }
        }
    });

    // set(self, value) -> Token
    cv_method!("set", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("set() requires self and value"));
        }
        let instance = &args[0];
        let value = args[1].clone();

        let name = {
            let borrowed = instance.borrow();
            if let PyObject::Instance { dict, .. } = &*borrowed {
                dict.get_str("_name")
                    .ok_or_else(|| PyError::type_error("ContextVar instance has no _name"))?
                    .str()
            } else {
                return Err(PyError::type_error("not a ContextVar instance"));
            }
        };

        // Get a new token ID
        let token_id = NEXT_TOKEN.with(|cell| {
            let mut n = cell.borrow_mut();
            let id = *n;
            *n += 1;
            id
        });

        // Push onto history stack
        CONTEXT_DATA.with(|cell| {
            let mut map = cell.borrow_mut();
            let stack = map.entry(name.clone()).or_insert_with(Vec::new);
            stack.push((token_id, value));
        });

        // Create a Token instance
        let mut token_dict = AttrMap::new();
        token_dict.insert_str("_token_id", py_int(token_id as i64));
        token_dict.insert_str("_var_name", py_str(&name));
        let token = PyObjectRef::new(PyObject::Instance {
            typ: TOKEN_TYPE
                .with(|cell| cell.borrow().clone())
                .ok_or_else(|| PyError::runtime_error("Token type not initialized".to_string()))?,
            dict: token_dict,
        });
        Ok(token)
    });

    // reset(self, token)
    cv_method!("reset", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("reset() requires self and token"));
        }
        let instance = &args[0];
        let token = &args[1];

        // Extract the token ID from the token instance
        let token_id = {
            let borrowed = token.borrow();
            if let PyObject::Instance { dict, .. } = &*borrowed {
                dict.get_str("_token_id")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(-1) as u64
            } else {
                return Err(PyError::type_error("reset() argument must be a Token"));
            }
        };

        // Extract the variable name
        let name = {
            let borrowed = instance.borrow();
            if let PyObject::Instance { dict, .. } = &*borrowed {
                dict.get_str("_name")
                    .ok_or_else(|| PyError::type_error("ContextVar instance has no _name"))?
                    .str()
            } else {
                return Err(PyError::type_error("not a ContextVar instance"));
            }
        };

        // Pop from history until we find the matching token
        CONTEXT_DATA.with(|cell| {
            let mut map = cell.borrow_mut();
            if let Some(stack) = map.get_mut(&name) {
                while let Some((tid, _)) = stack.last() {
                    if *tid == token_id {
                        stack.pop();
                        if stack.is_empty() {
                            map.remove(&name);
                        }
                        return;
                    }
                    stack.pop();
                }
            }
        });

        Ok(py_none())
    });

    // Create the ContextVar Type object
    let contextvar_type = PyObjectRef::new(PyObject::Type {
        name: "ContextVar".to_string(),
        dict: Box::new(str_map_to_typedict(contextvar_type_dict)),
        bases: vec![],
        mro: vec![],
    });

    // ---- Token type ----
    let token_type = PyObjectRef::new(PyObject::Type {
        name: "Token".to_string(),
        dict: {
            let mut td: crate::object::TypeDict = Default::default();
            // __repr__ for debugging
            td.insert_str(
                "__repr__",
                PyObjectRef::new(PyObject::BuiltinFunction {
                    name: "__repr__".to_string(),
                    func: |args| {
                        if args.is_empty() {
                            return Ok(py_str("<Token>"));
                        }
                        let borrowed = args[0].borrow();
                        if let PyObject::Instance { dict, .. } = &*borrowed {
                            if let Some(tid) = dict.get_str("_token_id") {
                                return Ok(py_str(&format!(
                                    "<Token var={:?} id={}>",
                                    dict.get_str("_var_name")
                                        .map(|v| v.str())
                                        .unwrap_or_default(),
                                    tid.as_i64().unwrap_or(-1)
                                )));
                            }
                        }
                        Ok(py_str("<Token>"))
                    },
                }),
            );
            td.insert_str("__name__", py_str("Token"));
            Box::new(td)
        },
        bases: vec![],
        mro: vec![],
    });

    // Store Token type in thread_local for the set() method to use
    thread_local! {
        static TOKEN_TYPE: RefCell<Option<PyObjectRef>> = const { RefCell::new(None) };
    }
    TOKEN_TYPE.with(|cell| {
        *cell.borrow_mut() = Some(token_type.clone());
    });

    // ---- copy_context() function ----
    let copy_context_func = PyObjectRef::new(PyObject::BuiltinFunction {
        name: "copy_context".to_string(),
        func: |_args| {
            // Build a dict with all current context variable values
            let mut context_vals = HashMap::new();
            CONTEXT_DATA.with(|cell| {
                let map = cell.borrow();
                for (name, stack) in map.iter() {
                    if let Some((_, val)) = stack.last() {
                        context_vals.insert(name.clone(), val.clone());
                    }
                }
            });

            // Create a module-like object that acts as a Context
            let mut ctx_module_dict = HashMap::new();
            for (k, v) in &context_vals {
                ctx_module_dict.insert(k.clone(), v.clone());
            }
            ctx_module_dict.insert_str("__name__", py_str("Context"));

            // Add items() method using Closure so we can capture context_vals
            let items_vals = context_vals.clone();
            ctx_module_dict.insert_str(
                "items",
                PyObjectRef::new(PyObject::Closure(Rc::new(move |_args| {
                    let mut items = Vec::new();
                    for (k, v) in items_vals.iter() {
                        items.push(py_tuple(vec![py_str(k), v.clone()]));
                    }
                    Ok(py_list(items))
                }))),
            );

            Ok(PyObjectRef::new(PyObject::Module {
                name: "Context".to_string(),
                dict: Box::new(str_map_to_typedict(ctx_module_dict)),
            }))
        },
    });

    // ---- Module contents ----
    d.insert_str("ContextVar", contextvar_type);
    d.insert_str("Token", token_type);
    d.insert_str("copy_context", copy_context_func);
    d.insert_str("__name__", py_str("contextvars"));
    d.insert_str("__doc__", py_str("Context Variables (thread-local stub)"));

    d
}

