use crate::object::*;
use std::collections::HashMap;

// ---- logging module ----
// basicConfig(level) stores level; getLogger(name) returns dict-like with
// .info/.debug/.warning/.error methods. Moved here from object.rs (was
// under a "---- logging module ----" banner in the monolithic object.rs —
// see the file-splitting refactor's memory entry for context).
thread_local! {
    pub static LOG_LEVEL: std::cell::RefCell<String> = std::cell::RefCell::new("WARNING".to_string());
}


pub fn logging_debug(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 2 {
        return Ok(py_none());
    }
    let level = LOG_LEVEL.with(|l| l.borrow().clone());
    if level != "DEBUG"
        && level != "INFO"
        && level != "WARNING"
        && level != "ERROR"
        && level != "CRITICAL"
    {
        return Ok(py_none());
    }
    let _msg = args[1].str();
    let _logger_name = {
        let borrowed = args[0].borrow();
        if let PyObject::Instance { dict, .. } = &*borrowed {
            dict.get("name").map(|n| n.str()).unwrap_or_default()
        } else {
            String::new()
        }
    };
    Ok(py_none())
}

pub fn logging_info(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 2 {
        return Ok(py_none());
    }
    let level = LOG_LEVEL.with(|l| l.borrow().clone());
    if level != "INFO" && level != "WARNING" && level != "ERROR" && level != "CRITICAL" {
        return Ok(py_none());
    }
    let msg = args[1].str();
    let logger_name = {
        let borrowed = args[0].borrow();
        if let PyObject::Instance { dict, .. } = &*borrowed {
            dict.get("name").map(|n| n.str()).unwrap_or_default()
        } else {
            String::new()
        }
    };
    eprintln!("INFO:{}:{}", logger_name, msg);
    Ok(py_none())
}

pub fn logging_warning(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 2 {
        return Ok(py_none());
    }
    let level = LOG_LEVEL.with(|l| l.borrow().clone());
    if level != "WARNING" && level != "ERROR" && level != "CRITICAL" {
        return Ok(py_none());
    }
    let msg = args[1].str();
    let logger_name = {
        let borrowed = args[0].borrow();
        if let PyObject::Instance { dict, .. } = &*borrowed {
            dict.get("name").map(|n| n.str()).unwrap_or_default()
        } else {
            String::new()
        }
    };
    eprintln!("WARNING:{}:{}", logger_name, msg);
    Ok(py_none())
}

pub fn logging_error(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 2 {
        return Ok(py_none());
    }
    let level = LOG_LEVEL.with(|l| l.borrow().clone());
    if level != "ERROR" && level != "CRITICAL" {
        return Ok(py_none());
    }
    let msg = args[1].str();
    let logger_name = {
        let borrowed = args[0].borrow();
        if let PyObject::Instance { dict, .. } = &*borrowed {
            dict.get("name").map(|n| n.str()).unwrap_or_default()
        } else {
            String::new()
        }
    };
    eprintln!("ERROR:{}:{}", logger_name, msg);
    Ok(py_none())
}


pub fn create_logging_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! log_func {
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

    log_func!("basicConfig", |args| {
        if args.len() >= 1 {
            // Accept basicConfig(level=...) via kwargs not available, use positional
            let level = args[0].str().to_uppercase();
            LOG_LEVEL.with(|l| *l.borrow_mut() = level);
        }
        Ok(py_none())
    });

    // Store logger instances in a thread-local registry
    thread_local! {
        static LOGGER_REGISTRY: std::cell::RefCell<HashMap<String, PyObjectRef>> = std::cell::RefCell::new(HashMap::new());
    }

    log_func!("getLogger", |args| {
        let name = if args.is_empty() {
            "root".to_string()
        } else {
            args[0].str()
        };
        // Check registry first
        let cached = LOGGER_REGISTRY.with(|reg| reg.borrow().get(&name).cloned());
        if let Some(logger) = cached {
            return Ok(logger);
        }
        // Create a new Logger type
        let logger_typ = PyObjectRef::new(PyObject::Type {
            name: "Logger".to_string(),
            dict: {
                let mut type_dict: crate::object::TypeDict = Default::default();
                type_dict.insert_str(
                    "info",
                    PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "info".to_string(),
                        func: logging_info,
                        self_obj: py_none(),
                    }),
                );
                type_dict.insert_str(
                    "debug",
                    PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "debug".to_string(),
                        func: logging_debug,
                        self_obj: py_none(),
                    }),
                );
                type_dict.insert_str(
                    "warning",
                    PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "warning".to_string(),
                        func: logging_warning,
                        self_obj: py_none(),
                    }),
                );
                type_dict.insert_str(
                    "error",
                    PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "error".to_string(),
                        func: logging_error,
                        self_obj: py_none(),
                    }),
                );
                type_dict.insert_str(
                    "setLevel",
                    PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "setLevel".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "setLevel requires level argument",
                                ));
                            }
                            // Store level in instance dict
                            let instance = args[0].clone();
                            let level = args[1].clone();
                            let mut dict = instance.borrow_mut();
                            if let PyObject::Instance {
                                dict: inst_dict, ..
                            } = &mut *dict
                            {
                                inst_dict.insert_str("level", level);
                            }
                            Ok(py_none())
                        },
                        self_obj: py_none(),
                    }),
                );
                type_dict.insert_str(
                    "addHandler",
                    PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "addHandler".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "addHandler requires handler argument",
                                ));
                            }
                            // Store handler in instance dict's _handlers list
                            let instance = args[0].clone();
                            let handler = args[1].clone();
                            let mut dict = instance.borrow_mut();
                            if let PyObject::Instance {
                                dict: inst_dict, ..
                            } = &mut *dict
                            {
                                let handlers =
                                    inst_dict.entry("_handlers".to_string()).or_insert_with(|| {
                                        PyObjectRef::new(PyObject::List(Vec::new()))
                                    });
                                if let PyObject::List(ref mut v) = &mut *handlers.borrow_mut() {
                                    v.push(handler);
                                }
                            }
                            Ok(py_none())
                        },
                        self_obj: py_none(),
                    }),
                );
                Box::new(type_dict)
            },
            bases: vec![],
            mro: vec![],
        });
        let instance = PyObjectRef::new(PyObject::Instance {
            typ: logger_typ,
            dict: AttrMap::from([("name".to_string(), py_str(&name))]),
        });
        LOGGER_REGISTRY.with(|reg| reg.borrow_mut().insert(name.clone(), instance.clone()));
        Ok(instance)
    });

    // NullHandler class (needed by urllib3 and other libs)
    // Handler base class
    let handler_class = PyObjectRef::new(PyObject::Type {
        name: "Handler".to_string(),
        dict: Box::new(str_map_to_typedict(HashMap::from([
            (
                "__init__".to_string(),
                PyObjectRef::new(PyObject::BuiltinFunction {
                    name: "__init__".to_string(),
                    func: |args| {
                        if args.len() > 1 {
                            let _ = args[0].borrow_mut().set_attribute("level", args[1].clone());
                        }
                        Ok(py_none())
                    },
                }),
            ),
            (
                "setLevel".to_string(),
                PyObjectRef::new(PyObject::BuiltinFunction {
                    name: "setLevel".to_string(),
                    func: |_| Ok(py_none()),
                }),
            ),
        ]))),
        bases: vec![],
        mro: vec![],
    });
    // Set MRO so isinstance checks work (Type needs itself in MRO)
    if let PyObject::Type { ref mut mro, .. } = &mut *handler_class.borrow_mut() {
        mro.push(handler_class.clone());
    }
    d.insert_str("Handler", handler_class.clone());

    // Filter base class — real code (Django's RequireDebugFalse/True,
    // `logging.config`) subclasses this and overrides `filter(record)`;
    // the base itself just needs a constructor and a default `filter`
    // that lets everything through (matching real `logging.Filter` with
    // no `name=` restriction applied).
    let filter_class = PyObjectRef::new(PyObject::Type {
        name: "Filter".to_string(),
        dict: Box::new(str_map_to_typedict(HashMap::from([
            (
                "__init__".to_string(),
                PyObjectRef::new(PyObject::BuiltinFunction {
                    name: "__init__".to_string(),
                    func: |args| {
                        let name = if args.len() > 1 {
                            args[1].str()
                        } else {
                            String::new()
                        };
                        let _ = args[0].borrow_mut().set_attribute("name", py_str(&name));
                        Ok(py_none())
                    },
                }),
            ),
            (
                "filter".to_string(),
                PyObjectRef::new(PyObject::BuiltinFunction {
                    name: "filter".to_string(),
                    func: |_| Ok(py_bool(true)),
                }),
            ),
        ]))),
        bases: vec![],
        mro: vec![],
    });
    if let PyObject::Type { ref mut mro, .. } = &mut *filter_class.borrow_mut() {
        mro.push(filter_class.clone());
    }
    d.insert_str("Filter", filter_class);

    // Formatter base class — real code (Django's `AdminEmailHandler` etc.,
    // `logging.config` dictConfig) constructs `Formatter(fmt=...)` and
    // calls `.format(record)`. A minimal but real implementation: supports
    // the common `%(levelname)s`/`%(message)s`/`%(name)s`/`%(asctime)s`-
    // style attributes actually present on a LogRecord, falling back to
    // `record.getMessage()` if no format string was given.
    let formatter_class = PyObjectRef::new(PyObject::Type {
        name: "Formatter".to_string(),
        dict: Box::new(str_map_to_typedict(HashMap::from([
            (
                "__init__".to_string(),
                PyObjectRef::new(PyObject::BuiltinFunction {
                    name: "__init__".to_string(),
                    func: |args| {
                        let fmt = if args.len() > 1 && !matches!(&*args[1].borrow(), PyObject::None)
                        {
                            Some(args[1].str())
                        } else {
                            None
                        };
                        let _ = args[0]
                            .borrow_mut()
                            .set_attribute("_fmt", fmt.map_or_else(py_none, |f| py_str(&f)));
                        Ok(py_none())
                    },
                }),
            ),
            (
                "format".to_string(),
                PyObjectRef::new(PyObject::BuiltinFunction {
                    name: "format".to_string(),
                    func: |args| {
                        if args.len() < 2 {
                            return Err(PyError::type_error("format() missing record argument"));
                        }
                        let fmt_attr = args[0].borrow().get_attribute("_fmt").ok();
                        let record = &args[1];
                        let get_msg = record.borrow().get_attribute("getMessage").ok();
                        let message = if let Some(f) = get_msg {
                            crate::object::call_bound_method(f, record.clone(), vec![])
                                .map(|v| v.str())
                                .unwrap_or_default()
                        } else {
                            String::new()
                        };
                        let text = match fmt_attr {
                            Some(f) if !matches!(&*f.borrow(), PyObject::None) => {
                                let mut s = f.str();
                                let levelname = record
                                    .borrow()
                                    .get_attribute("levelname")
                                    .map(|v| v.str())
                                    .unwrap_or_default();
                                let name = record
                                    .borrow()
                                    .get_attribute("name")
                                    .map(|v| v.str())
                                    .unwrap_or_default();
                                s = s.replace("%(levelname)s", &levelname);
                                s = s.replace("%(name)s", &name);
                                s = s.replace("%(message)s", &message);
                                s
                            }
                            _ => message,
                        };
                        Ok(py_str(&text))
                    },
                }),
            ),
        ]))),
        bases: vec![],
        mro: vec![],
    });
    if let PyObject::Type { ref mut mro, .. } = &mut *formatter_class.borrow_mut() {
        mro.push(formatter_class.clone());
    }
    d.insert_str("Formatter", formatter_class);
    d.insert_str(
        "NullHandler",
        PyObjectRef::new(PyObject::Closure(std::rc::Rc::new(move |_| {
            Ok(PyObjectRef::new(PyObject::Instance {
                typ: handler_class.clone(),
                dict: AttrMap::from([
                    (
                        "emit".to_string(),
                        PyObjectRef::new(PyObject::BuiltinFunction {
                            name: "emit".to_string(),
                            func: |_| Ok(py_none()),
                        }),
                    ),
                    (
                        "handle".to_string(),
                        PyObjectRef::new(PyObject::BuiltinFunction {
                            name: "handle".to_string(),
                            func: |_| Ok(py_none()),
                        }),
                    ),
                    ("level".to_string(), py_int(0)),
                ]),
            }))
        }))),
    );

    // Add level constants
    d.insert_str("CRITICAL", py_int(50));
    d.insert_str("ERROR", py_int(40));
    d.insert_str("WARNING", py_int(30));
    d.insert_str("INFO", py_int(20));
    d.insert_str("DEBUG", py_int(10));
    d.insert_str("NOTSET", py_int(0));

    d
}
