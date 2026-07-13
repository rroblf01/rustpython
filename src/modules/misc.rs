use crate::object::*;
use std::collections::HashMap;

pub fn create_re_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! re_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    re_func!("search", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("search() takes at least 2 arguments"));
        }
        let pattern = args[0].str();
        let string = args[1].str();
        match regex::Regex::new(&pattern) {
            Ok(re) => {
                match re.find(&string) {
                    Some(m) => {
                        let start = m.start();
                        let end = m.end();
                        let text = m.as_str().to_string();
                        Ok(py_tuple(vec![py_int(start as i64), py_int(end as i64), py_str(&text)]))
                    }
                    None => Ok(py_none()),
                }
            }
            Err(e) => Err(PyError::ValueError(format!("invalid regex: {}", e))),
        }
    });

    re_func!("match", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("match() takes at least 2 arguments"));
        }
        let pattern = args[0].str();
        let string = args[1].str();
        match regex::Regex::new(&pattern) {
            Ok(re) => {
                match re.find_at(&string, 0) {
                    Some(m) if m.start() == 0 => {
                        let end = m.end();
                        let text = m.as_str().to_string();
                        Ok(py_tuple(vec![py_int(0), py_int(end as i64), py_str(&text)]))
                    }
                    _ => Ok(py_none()),
                }
            }
            Err(e) => Err(PyError::ValueError(format!("invalid regex: {}", e))),
        }
    });

    re_func!("findall", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("findall() takes at least 2 arguments"));
        }
        let pattern = args[0].str();
        let string = args[1].str();
        match regex::Regex::new(&pattern) {
            Ok(re) => {
                let results: Vec<PyObjectRef> = re.find_iter(&string)
                    .map(|m| py_str(m.as_str()))
                    .collect();
                Ok(py_list(results))
            }
            Err(e) => Err(PyError::ValueError(format!("invalid regex: {}", e))),
        }
    });

    re_func!("sub", |args| {
        if args.len() < 3 {
            return Err(PyError::type_error("sub() takes at least 3 arguments"));
        }
        let pattern = args[0].str();
        let repl = args[1].str();
        let string = args[2].str();
        match regex::Regex::new(&pattern) {
            Ok(re) => {
                let result = re.replace_all(&string, repl.as_str());
                Ok(py_str(&result))
            }
            Err(e) => Err(PyError::ValueError(format!("invalid regex: {}", e))),
        }
    });

    re_func!("split", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("split() takes at least 2 arguments"));
        }
        let pattern = args[0].str();
        let string = args[1].str();
        let limit = if args.len() > 2 { args[2].as_i64().unwrap_or(0) as usize } else { 0 };
        match regex::Regex::new(&pattern) {
            Ok(re) => {
                let parts: Vec<PyObjectRef> = if limit > 0 {
                    re.splitn(&string, limit).map(|s| py_str(s)).collect()
                } else {
                    re.split(&string).map(|s| py_str(s)).collect()
                };
                Ok(py_list(parts))
            }
            Err(e) => Err(PyError::ValueError(format!("invalid regex: {}", e))),
        }
    });

    re_func!("compile", |args| {
        if args.len() < 1 {
            return Err(PyError::type_error("compile() takes at least 1 argument"));
        }
        let pattern = args[0].str();
        let flags = if args.len() > 1 { args[1].as_i64().unwrap_or(0) as i32 } else { 0 };
        match regex::Regex::new(&pattern) {
            Ok(re) => {
                Ok(PyObjectRef::new(PyObject::CompiledRegex {
                    regex: re,
                    pattern: pattern.to_string(),
                    flags,
                }))
            }
            Err(e) => Err(PyError::ValueError(format!("invalid regex: {}", e))),
        }
    });

    d
}

pub fn create_threading_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! thr_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    thr_func!("Thread", |args| {
        let target = if args.len() > 0 { args[0].clone() } else { py_none() };
        let thread_args = if args.len() > 1 {
            if let PyObject::Tuple(items) = &*args[1].borrow() {
                items.clone()
            } else { vec![] }
        } else { vec![] };
        let inner = std::sync::Arc::new(std::sync::Mutex::new(ThreadInner {
            handle: None,
            result: std::sync::Arc::new(std::sync::Mutex::new(None)),
            target,
            args: thread_args,
        }));
        Ok(PyObjectRef::new(PyObject::Thread(inner)))
    });

    thr_func!("Lock", |_| {
        let inner = std::sync::Arc::new(std::sync::Mutex::new(LockInner {
            lock: std::sync::atomic::AtomicBool::new(false),
        }));
        Ok(PyObjectRef::new(PyObject::Lock(inner)))
    });

    thr_func!("RLock", |_| {
        let inner = std::sync::Arc::new(std::sync::Mutex::new(RLockInner {
            owner: None,
            count: 0,
        }));
        Ok(PyObjectRef::new(PyObject::RLock(inner)))
    });

    thr_func!("Event", |_| {
        let inner = std::sync::Arc::new(EventInner {
            flag: std::sync::Mutex::new(false),
            condvar: std::sync::Condvar::new(),
        });
        Ok(PyObjectRef::new(PyObject::Event(inner)))
    });

    thr_func!("current_thread", |_| {
        Ok(py_str("MainThread"))
    });

    thr_func!("active_count", |_| {
        Ok(py_int(1))
    });

    d
}

pub fn create_weakref_weak_val_dict() -> PyObjectRef {
    PyObjectRef::new(PyObject::BuiltinFunction {
        name: "WeakValueDictionary".to_string(),
        func: |args| {
            if args.len() > 0 {
                // Copy items from the argument
                if let PyObject::Dict(items) = &*args[0].borrow() {
                    let mut new_dict = PyDict::new();
                    for (k, v) in items.items() {
                        let _ = new_dict.set(k, v);
                    }
                    return Ok(PyObjectRef::new(PyObject::Dict(new_dict)));
                }
            }
            Ok(py_dict())
        },
    })
}

pub fn create_weakref_weak_key_dict() -> PyObjectRef {
    PyObjectRef::new(PyObject::BuiltinFunction {
        name: "WeakKeyDictionary".to_string(),
        func: |args| {
            if args.len() > 0 {
                if let PyObject::Dict(items) = &*args[0].borrow() {
                    let mut new_dict = PyDict::new();
                    for (k, v) in items.items() {
                        let _ = new_dict.set(k, v);
                    }
                    return Ok(PyObjectRef::new(PyObject::Dict(new_dict)));
                }
            }
            Ok(py_dict())
        },
    })
}

pub fn create_weakref_weak_set() -> PyObjectRef {
    PyObjectRef::new(PyObject::BuiltinFunction {
        name: "WeakSet".to_string(),
        func: |args| {
            if args.len() > 0 {
                if let PyObject::Set(s) = &*args[0].borrow() {
                    return Ok(args[0].clone());
                }
                if let PyObject::List(items) = &*args[0].borrow() {
                    let mut s = PySet::new();
                    for item in items {
                        let _ = s.add(item.clone());
                    }
                    return Ok(PyObjectRef::new(PyObject::Set(s)));
                }
            }
            Ok(PyObjectRef::new(PyObject::Set(PySet::new())))
        },
    })
}

pub fn create_copy_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! copy_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    copy_func!("copy", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("copy() missing required argument"));
        }
        let obj = &args[0];
        let borrowed = obj.borrow();
        match &*borrowed {
            PyObject::None => Ok(py_none()),
            PyObject::Bool(b) => Ok(py_bool(*b)),
            PyObject::Int(_) | PyObject::Float(_) | PyObject::Str(_) | PyObject::Bytes(_) => Ok(obj.clone()),
            PyObject::Tuple(items) => {
                let mut new_items = Vec::with_capacity(items.len());
                for item in items {
                    new_items.push(item.clone());
                }
                Ok(PyObjectRef::imm(PyObject::Tuple(new_items)))
            }
            PyObject::List(items) => {
                let new_items: Vec<PyObjectRef> = items.iter().map(|i| {
                    // Shallow copy: clone references
                    let b = i.borrow();
                    match &*b {
                        PyObject::None => py_none(),
                        PyObject::Bool(b) => py_bool(*b),
                        PyObject::Int(_) | PyObject::Float(_) | PyObject::Str(_) => i.clone(),
                        _ => i.clone(),
                    }
                }).collect();
                Ok(py_list(new_items))
            }
            PyObject::Dict(dict) => {
                let mut new_dict = PyDict::new();
                for (k, v) in dict.items() {
                    let _ = new_dict.set(k, v);
                }
                Ok(PyObjectRef::new(PyObject::Dict(new_dict)))
            }
            PyObject::Set(s) => {
                let mut new_set = PySet::new();
                for item in s.to_vec() {
                    let _ = new_set.add(item);
                }
                Ok(PyObjectRef::new(PyObject::Set(new_set)))
            }
            _ => {
                // For instances and custom types, try __copy__
                if let Ok(copy_method) = borrowed.get_attribute("__copy__") {
                    drop(borrowed);
                    return crate::object::call_function(&copy_method, vec![obj.clone()]);
                }
                Ok(obj.clone())
            }
        }
    });

    copy_func!("deepcopy", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("deepcopy() missing required argument"));
        }
        let obj = &args[0];
        let memo = if args.len() > 1 { args[1].clone() } else { py_dict() };
        let borrowed = obj.borrow();
        match &*borrowed {
            PyObject::None => Ok(py_none()),
            PyObject::Bool(b) => Ok(py_bool(*b)),
            PyObject::Int(_) | PyObject::Float(_) | PyObject::Str(_) | PyObject::Bytes(_) => Ok(obj.clone()),
            PyObject::Tuple(items) => {
                let mut new_items = Vec::with_capacity(items.len());
                for item in items {
                    new_items.push(deepcopy_one(item, &memo)?);
                }
                Ok(PyObjectRef::imm(PyObject::Tuple(new_items)))
            }
            PyObject::List(items) => {
                let mut new_items = Vec::with_capacity(items.len());
                for item in items {
                    new_items.push(deepcopy_one(item, &memo)?);
                }
                Ok(py_list(new_items))
            }
            PyObject::Dict(dict) => {
                let mut new_dict = PyDict::new();
                for (k, v) in dict.items() {
                    let new_k = deepcopy_one(&k, &memo)?;
                    let new_v = deepcopy_one(&v, &memo)?;
                    let _ = new_dict.set(new_k, new_v);
                }
                Ok(PyObjectRef::new(PyObject::Dict(new_dict)))
            }
            _ => {
                // For instances, try __deepcopy__ first
                if let Ok(dc_method) = borrowed.get_attribute("__deepcopy__") {
                    drop(borrowed);
                    return crate::object::call_function(&dc_method, vec![obj.clone(), memo]);
                }
                Ok(obj.clone())
            }
        }
    });

    // Error class
    d.insert("Error".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "Error".to_string(),
        func: |args| {
            let msg = if !args.is_empty() { args[0].str() } else { "copy error".to_string() };
            Err(PyError::Exception(msg, py_none()))
        },
    }));

    d
}

pub fn create_weakref_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! wr_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    // ref(obj) returns a weak reference object (callable)
    // If the object is still alive, calling it returns the object
    // Since we don't have full GC, we use a simple Rc-based weak reference
    wr_func!("ref", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("ref() requires at least 1 argument"));
        }
        let obj = args[0].clone();
        // Return a BuiltinMethod that when called returns the original object
        Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
            name: "weakref".to_string(),
            func: |args| Ok(args[0].clone()),
            self_obj: obj,
        }))
    });

    wr_func!("proxy", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("proxy() requires at least 1 argument"));
        }
        Ok(args[0].clone())
    });

    wr_func!("getweakrefcount", |_| Ok(py_int(0)));
    wr_func!("getweakrefs", |_| Ok(py_list(vec![])));

    // Type constants
    d.insert("ReferenceType".to_string(), py_str("weakref"));
    d.insert("ProxyType".to_string(), py_str("weakproxy"));
    d.insert("CallableProxyType".to_string(), py_str("weakcallableproxy"));

    // Internal function used by weakrefset
    wr_func!("_remove_dead_weakref", |_| Ok(py_none()));

    d
}

pub fn create_collections_abc_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! abc_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    abc_func!("__import__", |_| Ok(py_bool(true)));

    // Abstract base classes as simple markers
    let abc_meta = PyObjectRef::new(PyObject::BuiltinFunction {
        name: "ABCMeta".to_string(),
        func: |args| {
            Ok(PyObjectRef::new(PyObject::Instance {
                typ: py_dict(), // simplified type
                dict: HashMap::new(),
            }))
        },
    });

    d.insert("ABCMeta".to_string(), abc_meta);
    d.insert("Hashable".to_string(), py_str("Hashable"));
    d.insert("Iterable".to_string(), py_str("Iterable"));
    d.insert("Iterator".to_string(), py_str("Iterator"));
    d.insert("Sized".to_string(), py_str("Sized"));
    d.insert("Callable".to_string(), py_str("Callable"));
    d.insert("Sequence".to_string(), py_str("Sequence"));
    d.insert("MutableSequence".to_string(), py_str("MutableSequence"));
    d.insert("Set".to_string(), py_str("Set"));
    d.insert("MutableSet".to_string(), py_str("MutableSet"));
    d.insert("Mapping".to_string(), py_str("Mapping"));
    d.insert("MutableMapping".to_string(), py_str("MutableMapping"));
    d.insert("MappingView".to_string(), py_str("MappingView"));
    d.insert("ItemsView".to_string(), py_str("ItemsView"));
    d.insert("KeysView".to_string(), py_str("KeysView"));
    d.insert("ValuesView".to_string(), py_str("ValuesView"));
    d.insert("Container".to_string(), py_str("Container"));

    d
}

pub fn create_types_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! t_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    t_func!("FunctionType", |args| {
        if args.is_empty() { return Err(PyError::type_error("FunctionType() requires an argument")); }
        Ok(args[0].clone())
    });
    t_func!("LambdaType", |args| {
        if args.is_empty() { return Err(PyError::type_error("LambdaType() requires an argument")); }
        Ok(args[0].clone())
    });
    t_func!("MethodType", |args| {
        if args.is_empty() { return Err(PyError::type_error("MethodType() requires an argument")); }
        Ok(args[0].clone())
    });
    t_func!("BuiltinFunctionType", |args| {
        if args.is_empty() { return Err(PyError::type_error("BuiltinFunctionType() requires an argument")); }
        Ok(args[0].clone())
    });
    t_func!("ModuleType", |args| {
        if args.is_empty() { return Err(PyError::type_error("ModuleType() requires an argument")); }
        Ok(args[0].clone())
    });
    t_func!("NoneType", |_| Ok(py_none()));
    t_func!("GeneratorType", |args| {
        if args.is_empty() { return Err(PyError::type_error("GeneratorType() requires an argument")); }
        Ok(args[0].clone())
    });
    t_func!("CoroutineType", |args| {
        if args.is_empty() { return Err(PyError::type_error("CoroutineType() requires an argument")); }
        Ok(args[0].clone())
    });
    t_func!("SimpleNamespace", |args| {
        let d = py_dict();
        if args.len() > 0 {
            if let PyObject::Dict(items) = &*args[0].borrow() {
                let mut new_dict = PyDict::new();
                for (k, v) in items.items() {
                    let _ = new_dict.set(k, v);
                }
                return Ok(PyObjectRef::new(PyObject::Dict(new_dict)));
            }
        }
        Ok(d)
    });
    d.insert("CodeType".to_string(), py_str("code"));
    d.insert("MappingProxyType".to_string(), py_str("mappingproxy"));

    d
}

pub fn create_struct_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! s_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    fn fmt_size(fmt: &str) -> usize {
        let mut size = 0usize;
        let mut chars = fmt.chars().peekable();
        while let Some(c) = chars.next() {
            let count = match c {
                ' ' | '!' | '@' | '<' | '>' | '=' => continue,
                '0'..='9' => {
                    let mut n = String::from(c);
                    while let Some(d @ '0'..='9') = chars.peek() {
                        n.push(*d);
                        chars.next();
                    }
                    n.parse::<usize>().unwrap_or(1)
                }
                _ => 1,
            };
            size += match c {
                'c' | 'b' | 'B' | '?' | 'x' => 1 * count,
                'h' | 'H' => 2 * count,
                'i' | 'I' | 'l' | 'L' | 'f' => 4 * count,
                'q' | 'Q' | 'd' => 8 * count,
                's' | 'p' => count,
                _ => 1 * count,
            };
        }
        size
    }

    s_func!("calcsize", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("calcsize() missing required argument"));
        }
        let fmt = args[0].str();
        Ok(py_int(fmt_size(&fmt) as i64))
    });

    s_func!("pack", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("pack() requires format string and values"));
        }
        let fmt = args[0].str();
        let values: Vec<u8> = args[1..].iter().map(|arg| {
            if let Some(n) = arg.as_i64() { n as u8 } else { 0u8 }
        }).collect();
        let size = fmt_size(&fmt);
        let mut data = Vec::with_capacity(size);
        for v in &values { data.push(*v); }
        while data.len() < size { data.push(0); }
        Ok(PyObjectRef::imm(PyObject::Bytes(data)))
    });

    s_func!("unpack", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("unpack() requires format string and buffer"));
        }
        let fmt = args[0].str();
        let buf = {
            let b = args[1].borrow();
            match &*b {
                PyObject::Bytes(data) => data.clone(),
                _ => return Err(PyError::type_error("unpack() arg 2 must be bytes")),
            }
        };
        let size = fmt_size(&fmt);
        if buf.len() < size {
            return Err(PyError::type_error(format!("unpack() requires a buffer of {} bytes", size)));
        }
        let values: Vec<PyObjectRef> = buf[..size].iter().map(|&b| py_int(b as i64)).collect();
        Ok(PyObjectRef::imm(PyObject::Tuple(values)))
    });

    d.insert("error".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "error".to_string(),
        func: |args| {
            let msg = if !args.is_empty() { args[0].str() } else { "struct error".to_string() };
            Err(PyError::Exception(msg, py_none()))
        },
    }));

    d
}

pub fn create_bisect_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! bisect_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    bisect_func!("bisect_left", |args| {
        if args.len() < 2 { return Err(PyError::type_error("bisect_left() requires at least 2 arguments (list, item)")); }
        let items = {
            let a = args[0].borrow();
            match &*a { PyObject::List(v) => v.clone(), _ => return Err(PyError::type_error("bisect_left() argument must be a list")) }
        };
        let x = &args[1];
        let mut lo = if args.len() > 2 { args[2].as_i64().ok_or_else(|| PyError::type_error("lo must be an integer"))? as usize } else { 0 };
        let mut hi = if args.len() > 3 { args[3].as_i64().ok_or_else(|| PyError::type_error("hi must be an integer"))? as usize } else { items.len() };
        while lo < hi {
            let mid = (lo + hi) / 2;
            if items[mid].borrow().lt(x)? { lo = mid + 1; } else { hi = mid; }
        }
        Ok(py_int(lo as i64))
    });

    bisect_func!("bisect_right", |args| {
        if args.len() < 2 { return Err(PyError::type_error("bisect_right() requires at least 2 arguments (list, item)")); }
        let items = {
            let a = args[0].borrow();
            match &*a { PyObject::List(v) => v.clone(), _ => return Err(PyError::type_error("bisect_right() argument must be a list")) }
        };
        let x = &args[1];
        let mut lo = if args.len() > 2 { args[2].as_i64().ok_or_else(|| PyError::type_error("lo must be an integer"))? as usize } else { 0 };
        let mut hi = if args.len() > 3 { args[3].as_i64().ok_or_else(|| PyError::type_error("hi must be an integer"))? as usize } else { items.len() };
        while lo < hi {
            let mid = (lo + hi) / 2;
            if x.borrow().lt(&items[mid])? { hi = mid; } else { lo = mid + 1; }
        }
        Ok(py_int(lo as i64))
    });

    bisect_func!("insort_left", |args| {
        if args.len() < 2 { return Err(PyError::type_error("insort_left() requires at least 2 arguments (list, item)")); }
        let items = {
            let a = args[0].borrow();
            match &*a { PyObject::List(v) => v.clone(), _ => return Err(PyError::type_error("insort_left() argument must be a list")) }
        };
        let x = &args[1];
        let mut lo = if args.len() > 2 { args[2].as_i64().ok_or_else(|| PyError::type_error("lo must be an integer"))? as usize } else { 0 };
        let mut hi = if args.len() > 3 { args[3].as_i64().ok_or_else(|| PyError::type_error("hi must be an integer"))? as usize } else { items.len() };
        while lo < hi {
            let mid = (lo + hi) / 2;
            if items[mid].borrow().lt(x)? { lo = mid + 1; } else { hi = mid; }
        }
        if let PyObject::List(list) = &mut *args[0].borrow_mut() { list.insert(lo, x.clone()); Ok(py_none()) }
        else { Err(PyError::type_error("insort_left() argument must be a list")) }
    });

    bisect_func!("insort_right", |args| {
        if args.len() < 2 { return Err(PyError::type_error("insort_right() requires at least 2 arguments (list, item)")); }
        let items = {
            let a = args[0].borrow();
            match &*a { PyObject::List(v) => v.clone(), _ => return Err(PyError::type_error("insort_right() argument must be a list")) }
        };
        let x = &args[1];
        let mut lo = if args.len() > 2 { args[2].as_i64().ok_or_else(|| PyError::type_error("lo must be an integer"))? as usize } else { 0 };
        let mut hi = if args.len() > 3 { args[3].as_i64().ok_or_else(|| PyError::type_error("hi must be an integer"))? as usize } else { items.len() };
        while lo < hi {
            let mid = (lo + hi) / 2;
            if x.borrow().lt(&items[mid])? { hi = mid; } else { lo = mid + 1; }
        }
        if let PyObject::List(list) = &mut *args[0].borrow_mut() { list.insert(lo, x.clone()); Ok(py_none()) }
        else { Err(PyError::type_error("insort_right() argument must be a list")) }
    });

    d
}

pub fn create_heapq_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! heap_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    // Internal: sift-down (for heappop, heapreplace, heapify)
    fn _siftdown(heap: &mut Vec<PyObjectRef>, start: usize, pos: usize) {
        let mut pos = pos;
        while pos > start {
            let parent = (pos - 1) / 2;
            if heap[pos].borrow().lt(&heap[parent]).unwrap_or(false) {
                heap.swap(pos, parent);
                pos = parent;
            } else {
                break;
            }
        }
    }

    // Internal: sift-up (for heapify)
    fn _siftup(heap: &mut Vec<PyObjectRef>, pos: usize) {
        let end = heap.len();
        let mut pos = pos;
        let start = pos;
        while pos < end {
            let left = 2 * pos + 1;
            let right = 2 * pos + 2;
            let mut smallest = pos;
            if left < end && heap[left].borrow().lt(&heap[smallest]).unwrap_or(false) {
                smallest = left;
            }
            if right < end && heap[right].borrow().lt(&heap[smallest]).unwrap_or(false) {
                smallest = right;
            }
            if smallest == pos { break; }
            heap.swap(pos, smallest);
            pos = smallest;
        }
        // Bubble back up if needed (after moving nodes)
        _siftdown(heap, start, pos);
    }

    heap_func!("heapify", |args| {
        if args.is_empty() { return Err(PyError::type_error("heapify() missing required argument")); }
        if let PyObject::List(list) = &mut *args[0].borrow_mut() {
            let n = list.len();
            if n > 1 {
                for i in (0..n / 2).rev() {
                    _siftup(list, i);
                }
            }
            Ok(py_none())
        } else {
            Err(PyError::type_error("heapify() argument must be a list"))
        }
    });

    heap_func!("heappush", |args| {
        if args.len() < 2 { return Err(PyError::type_error("heappush() requires 2 arguments (heap, item)")); }
        if let PyObject::List(list) = &mut *args[0].borrow_mut() {
            list.push(args[1].clone());
            _siftdown(list, 0, list.len() - 1);
            Ok(py_none())
        } else {
            Err(PyError::type_error("heappush() argument must be a list"))
        }
    });

    heap_func!("heappop", |args| {
        if args.is_empty() { return Err(PyError::type_error("heappop() missing required argument")); }
        if let PyObject::List(list) = &mut *args[0].borrow_mut() {
            if list.is_empty() { return Err(PyError::index_error("pop from an empty heap")); }
            let last = list.len() - 1;
            list.swap(0, last);
            let result = list.pop().unwrap();
            if !list.is_empty() { _siftup(list, 0); }
            Ok(result)
        } else {
            Err(PyError::type_error("heappop() argument must be a list"))
        }
    });

    heap_func!("heapreplace", |args| {
        if args.len() < 2 { return Err(PyError::type_error("heapreplace() requires 2 arguments (heap, item)")); }
        if let PyObject::List(list) = &mut *args[0].borrow_mut() {
            if list.is_empty() { return Err(PyError::index_error("heapreplace() on empty heap")); }
            let result = list[0].clone();
            list[0] = args[1].clone();
            _siftup(list, 0);
            Ok(result)
        } else {
            Err(PyError::type_error("heapreplace() argument must be a list"))
        }
    });

    // Helper: extract comparable values for nlargest/nsmallest
    fn _extract_items(args: &[PyObjectRef]) -> PyResult<(usize, Vec<PyObjectRef>)> {
        if args.len() < 2 { return Err(PyError::type_error("requires at least 2 arguments (n, iterable)")); }
        let n = args[0].as_i64().ok_or_else(|| PyError::type_error("n must be an integer"))?;
        if n < 0 { return Err(PyError::value_error("n must be non-negative")); }
        let n = n as usize;
        // Extract items from iterable
        let iterable = crate::object::builtin_iter(&[args[1].clone()])?;
        let mut items = Vec::new();
        loop {
            match crate::object::builtin_next(&[iterable.clone()]) {
                Ok(val) => items.push(val),
                Err(PyError::StopIteration) => break,
                Err(e) => return Err(e),
            }
        }
        Ok((n, items))
    }

    heap_func!("nlargest", |args| {
        let (n, mut items) = _extract_items(args)?;
        if n == 0 { return Ok(py_list(Vec::new())); }
        // Use a min-heap of size n to track largest n elements
        if items.len() <= n {
            // Sort descending
            items.sort_by(|a, b| b.borrow().lt(a).unwrap_or(false).cmp(&true).reverse());
            return Ok(py_list(items));
        }
        // Build a min-heap of the first n elements
        let mut heap: Vec<PyObjectRef> = items.drain(..n).collect();
        if heap.len() > 1 {
            for i in (0..heap.len() / 2).rev() {
                _siftup(&mut heap, i);
            }
        }
        for item in items {
            if item.borrow().lt(&heap[0]).unwrap_or(false) {
                // item < smallest in heap, skip
            } else {
                heap[0] = item;
                _siftup(&mut heap, 0);
            }
        }
        // Sort descending
        heap.sort_by(|a, b| b.borrow().lt(a).unwrap_or(false).cmp(&true).reverse());
        Ok(py_list(heap))
    });

    heap_func!("nsmallest", |args| {
        let (n, mut items) = _extract_items(args)?;
        if n == 0 { return Ok(py_list(Vec::new())); }
        if items.len() <= n {
            items.sort_by(|a, b| a.borrow().lt(b).unwrap_or(false).cmp(&true));
            return Ok(py_list(items));
        }
        // Use a max-heap (negation) of size n to track smallest n elements
        // Actually, we can use a max-heap: track largest in the small set
        // For max-heap we invert comparison
        let mut heap: Vec<PyObjectRef> = items.drain(..n).collect();
        if heap.len() > 1 {
            for i in (0..heap.len() / 2).rev() {
                _siftup_max(&mut heap, i);
            }
        }
        for item in items {
            if heap[0].borrow().lt(&item).unwrap_or(false) {
                // item < heap[0], skip
            } else {
                heap[0] = item;
                _siftup_max(&mut heap, 0);
            }
        }
        heap.sort_by(|a, b| a.borrow().lt(b).unwrap_or(false).cmp(&true));
        Ok(py_list(heap))
    });

    fn _siftup_max(heap: &mut Vec<PyObjectRef>, pos: usize) {
        let end = heap.len();
        let mut pos = pos;
        while pos < end {
            let left = 2 * pos + 1;
            let right = 2 * pos + 2;
            let mut largest = pos;
            if left < end && heap[largest].borrow().lt(&heap[left]).unwrap_or(false) {
                largest = left;
            }
            if right < end && heap[largest].borrow().lt(&heap[right]).unwrap_or(false) {
                largest = right;
            }
            if largest == pos { break; }
            heap.swap(pos, largest);
            pos = largest;
        }
    }

    d
}

pub fn create_enum_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! enum_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    enum_func!("auto", |args| {
        let _ = args;
        let val = ENUM_AUTO_COUNTER.fetch_add(1, Ordering::SeqCst);
        Ok(py_int(val))
    });

    enum_func!("Enum", |args| {
        if args.is_empty() { return Err(PyError::type_error("Enum() requires at least 1 argument")); }
        // Enum is a callable that returns an int.
        // If called with (name, value), return value; if called with just value, return it.
        if args.len() >= 2 {
            Ok(args[1].clone())
        } else {
            Ok(args[0].clone())
        }
    });

    enum_func!("IntEnum", |args| {
        if args.is_empty() { return Err(PyError::type_error("IntEnum() requires at least 1 argument")); }
        if args.len() >= 2 {
            Ok(args[1].clone())
        } else {
            Ok(args[0].clone())
        }
    });

    d
}

pub fn create_uuid_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! uuid_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    uuid_func!("uuid4", |args| {
        if !args.is_empty() { return Err(PyError::type_error("uuid4() takes no arguments")); }
        let r1 = fast_random_u64();
        let r2 = fast_random_u64();
        let time_low = r1 as u32;
        let time_mid = (r1 >> 32) as u16;
        let time_hi_and_version = ((r1 >> 48) as u16 & 0x0FFF) | 0x4000;
        let clock_seq = (r2 as u16 & 0x3FFF) | 0x8000;
        let node = (r2 >> 16) as u64;
        Ok(py_str(&format!("{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
            time_low, time_mid, time_hi_and_version, clock_seq, node)))
    });

    d
}

pub fn create_csv_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! csv_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    csv_func!("reader", |args| {
        if args.is_empty() { return Err(PyError::type_error("reader() missing required argument")); }
        let s = args[0].str();
        let mut result = Vec::new();
        for line in s.lines() {
            if line.trim().is_empty() { continue; }
            let fields: Vec<PyObjectRef> = line.split(',').map(|f| py_str(f.trim())).collect();
            result.push(py_list(fields));
        }
        Ok(py_list(result))
    });

    csv_func!("writer", |args| {
        if args.is_empty() { return Err(PyError::type_error("writer() missing required argument")); }
        let data = &args[0];
        let borrowed = data.borrow();
        if let PyObject::List(rows) = &*borrowed {
            let mut lines = Vec::new();
            for row in rows {
                let row_b = row.borrow();
                if let PyObject::List(fields) = &*row_b {
                    let line: Vec<String> = fields.iter().map(|f| f.str()).collect();
                    lines.push(line.join(","));
                } else {
                    return Err(PyError::type_error("writer() argument must be a list of lists"));
                }
            }
            Ok(py_str(&lines.join("\n")))
        } else {
            Err(PyError::type_error("writer() argument must be a list of lists"))
        }
    });

    d
}

pub fn create_io_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! io_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    io_func!("StringIO", |args| {
        // Create the type on each call (no caching needed for native modules)
        let mut type_dict = HashMap::new();
        type_dict.insert("read".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "read".to_string(), func: io_stringio_read }));
        type_dict.insert("readline".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "readline".to_string(), func: io_stringio_readline }));
        type_dict.insert("write".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "write".to_string(), func: io_stringio_write }));
        type_dict.insert("seek".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "seek".to_string(), func: io_stringio_seek }));
        type_dict.insert("tell".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "tell".to_string(), func: io_stringio_tell }));
        type_dict.insert("getvalue".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "getvalue".to_string(), func: io_stringio_getvalue }));
        let typ = PyObjectRef::new(PyObject::Type {
            name: "StringIO".to_string(),
            dict: type_dict,
            bases: vec![],
            mro: vec![],
        });
        let mut instance_dict = HashMap::new();
        let initial = if !args.is_empty() { args[0].str() } else { String::new() };
        instance_dict.insert("_buffer".to_string(), py_str(&initial));
        instance_dict.insert("_pos".to_string(), py_int(0));
        Ok(PyObjectRef::new(PyObject::Instance {
            typ,
            dict: instance_dict,
        }))
    });

    io_func!("BytesIO", |args| {
        let mut type_dict = HashMap::new();
        type_dict.insert("read".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "read".to_string(), func: io_bytesio_read }));
        type_dict.insert("readline".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "readline".to_string(), func: io_bytesio_readline }));
        type_dict.insert("write".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "write".to_string(), func: io_bytesio_write }));
        type_dict.insert("seek".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "seek".to_string(), func: io_bytesio_seek }));
        type_dict.insert("tell".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "tell".to_string(), func: io_bytesio_tell }));
        type_dict.insert("getvalue".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "getvalue".to_string(), func: io_bytesio_getvalue }));
        let typ = PyObjectRef::new(PyObject::Type {
            name: "BytesIO".to_string(),
            dict: type_dict,
            bases: vec![],
            mro: vec![],
        });
        let mut instance_dict = HashMap::new();
        let initial = if !args.is_empty() {
            let a = args[0].borrow();
            match &*a {
                PyObject::Bytes(b) => b.clone(),
                _ => vec![],
            }
        } else { vec![] };
        instance_dict.insert("_buffer".to_string(), PyObjectRef::imm(PyObject::Bytes(initial)));
        instance_dict.insert("_pos".to_string(), py_int(0));
        Ok(PyObjectRef::new(PyObject::Instance {
            typ,
            dict: instance_dict,
        }))
    });

    d
}

pub fn create_contextlib_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! ctx_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }
    ctx_func!("contextmanager", |args| {
        if args.is_empty() { return Err(PyError::type_error("contextmanager() missing argument")); }
        Ok(args[0].clone())
    });
    ctx_func!("nullcontext", |args| {
        if args.is_empty() { Ok(py_none()) } else { Ok(args[0].clone()) }
    });
    ctx_func!("suppress", |_| Ok(py_none()));
    d
}

pub fn create_platform_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! plat_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }
    plat_func!("platform", |_| {
        let os = std::env::consts::OS;
        let arch = std::env::consts::ARCH;
        Ok(py_str(&format!("{}-{}", os, arch)))
    });
    plat_func!("machine", |_| {
        Ok(py_str(std::env::consts::ARCH))
    });
    plat_func!("processor", |_| {
        // Fall back to architecture string if no more specific info
        Ok(py_str(std::env::consts::ARCH))
    });
    plat_func!("python_implementation", |_| {
        Ok(py_str("RustPython"))
    });
    plat_func!("python_version", |_| {
        Ok(py_str("3.12.0"))
    });
    plat_func!("system", |_| {
        Ok(py_str(std::env::consts::OS))
    });
    d
}

pub fn create_getopt_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! getopt_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    // Helper: check if a short option expects an argument (followed by ':' in shortopts)
    fn short_has_arg(c: char, shortopts: &str) -> bool {
        if let Some(pos) = shortopts.find(c) {
            shortopts.as_bytes().get(pos + 1) == Some(&b':')
        } else {
            false
        }
    }

    getopt_func!("getopt", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("getopt() requires at least 2 arguments (args, shortopts)"));
        }
        let shortopts = args[1].str();
        // Parse longopts if provided (third argument is a list of long option names)
        let longopts: Vec<String> = if args.len() > 2 {
            if let PyObject::List(list) = &*args[2].borrow() {
                list.iter().map(|s| s.str()).collect()
            } else {
                return Err(PyError::type_error("longopts must be a list"));
            }
        } else {
            Vec::new()
        };

        // Extract the argument list from the first argument (should be a list of strings)
        let arg_list: Vec<String> = if let PyObject::List(list) = &*args[0].borrow() {
            list.iter().map(|s| s.str()).collect()
        } else {
            return Err(PyError::type_error("args must be a list"));
        };

        let mut opts: Vec<PyObjectRef> = Vec::new();
        let mut positional: Vec<PyObjectRef> = Vec::new();
        let mut i: usize = 1; // Skip the program name (args[0])
        let mut options_done = false;

        while i < arg_list.len() {
            let arg = &arg_list[i];
            if options_done || !arg.starts_with('-') {
                positional.push(py_str(arg));
                i += 1;
                if arg.starts_with('-') { options_done = true; }
                continue;
            }
            if arg == "--" {
                options_done = true;
                i += 1;
                continue;
            }
            if arg.starts_with("--") {
                // Long option
                let opt_name = &arg[2..];
                let (name, val) = if let Some(eq_pos) = opt_name.find('=') {
                    (&opt_name[..eq_pos], Some(&opt_name[eq_pos + 1..]))
                } else {
                    (opt_name, None)
                };
                // Check if this long option expects an argument
                let needs_val = longopts.iter().any(|lo| {
                    let base = if lo.ends_with('=') { &lo[..lo.len()-1] } else { lo.as_str() };
                    base == name && lo.ends_with('=')
                });
                match val {
                    Some(v) => opts.push(py_tuple(vec![py_str(&format!("--{}", name)), py_str(v)])),
                    None => {
                        if needs_val {
                            i += 1;
                            if i < arg_list.len() {
                                opts.push(py_tuple(vec![py_str(&format!("--{}", name)), py_str(&arg_list[i])]));
                            } else {
                                return Err(PyError::type_error(&format!("option --{} requires a value", name)));
                            }
                        } else {
                            opts.push(py_tuple(vec![py_str(&format!("--{}", name)), py_str("")]));
                        }
                    }
                }
                i += 1;
            } else {
                // Short option(s)
                let chars: Vec<char> = arg[1..].chars().collect();
                for (j, c) in chars.iter().enumerate() {
                    if !shortopts.contains(*c) {
                        return Err(PyError::type_error(&format!("option -{} not recognized", c)));
                    }
                    if short_has_arg(*c, &shortopts) {
                        if j + 1 < chars.len() {
                            // Value attached: -xvalue
                            let val: String = chars[j + 1..].iter().collect();
                            opts.push(py_tuple(vec![py_str(&format!("-{}", c)), py_str(&val)]));
                            break;
                        } else {
                            i += 1;
                            if i < arg_list.len() {
                                opts.push(py_tuple(vec![py_str(&format!("-{}", c)), py_str(&arg_list[i])]));
                            } else {
                                return Err(PyError::type_error(&format!("option -{} requires an argument", c)));
                            }
                        }
                    } else {
                        opts.push(py_tuple(vec![py_str(&format!("-{}", c)), py_str("")]));
                    }
                }
                i += 1;
            }
        }

        Ok(py_tuple(vec![py_list(opts), py_list(positional)]))
    });
    d
}

pub fn create_getpass_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! getpass_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }
    getpass_func!("getuser", |_| {
        let user = std::env::var("USER")
            .or_else(|_| std::env::var("LOGNAME"))
            .unwrap_or_else(|_| "unknown".to_string());
        Ok(py_str(&user))
    });
    getpass_func!("getpass", |args| {
        let prompt = if args.is_empty() { "Password: ".to_string() } else { args[0].str() };
        // In this minimal native implementation, we echo the prompt and read a line from stdin.
        // This is simplified — a real getpass would disable terminal echo.
        print!("{}", prompt);
        use std::io::Write;
        std::io::stdout().flush().ok();
        let mut password = String::new();
        match std::io::stdin().read_line(&mut password) {
            Ok(_) => Ok(py_str(password.trim_end())),
            Err(_) => Err(PyError::runtime_error("failed to read password")),
        }
    });
    d
}

pub fn create_graphlib_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! gl_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    // TopologicalSorter — returns a sorted list from the given graph
    gl_func!("TopologicalSorter", |args| {
        // Accept a dict or list of pairs as graph
        let mut edges: Vec<(String, String)> = Vec::new();
        if args.len() >= 1 {
            let graph = &args[0];
            let borrowed = graph.borrow();
            match &*borrowed {
                PyObject::Dict(dict) => {
                    for (key, val) in dict.items() {
                        let k = key.str();
                        let v = val.str();
                        edges.push((k, v));
                    }
                }
                PyObject::List(items) => {
                    for item in items {
                        if let PyObject::Tuple(pair) = &*item.borrow() {
                            if pair.len() >= 2 {
                                edges.push((pair[0].str(), pair[1].str()));
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        // Simple topological sort: Kahn's algorithm
        let mut in_degree: HashMap<String, usize> = HashMap::new();
        let mut adj: HashMap<String, Vec<String>> = HashMap::new();
        let mut nodes: std::collections::HashSet<String> = std::collections::HashSet::new();

        for (from, to) in &edges {
            adj.entry(from.clone()).or_default().push(to.clone());
            in_degree.entry(to.clone()).or_insert(0);
            in_degree.entry(from.clone()).or_insert(0);
            nodes.insert(from.clone());
            nodes.insert(to.clone());
        }

        // Also handle nodes referenced only as keys/values in pairs
        for n in &edges {
            in_degree.entry(n.0.clone()).or_insert(0);
            in_degree.entry(n.1.clone()).or_insert(0);
            nodes.insert(n.0.clone());
            nodes.insert(n.1.clone());
        }

        for (_, neighbors) in &adj {
            for n in neighbors {
                *in_degree.entry(n.clone()).or_insert(0) += 1;
            }
        }

        let mut queue: Vec<String> = Vec::new();
        for (node, deg) in &in_degree {
            if *deg == 0 {
                queue.push(node.clone());
            }
        }

        let mut sorted: Vec<PyObjectRef> = Vec::new();
        while !queue.is_empty() {
            queue.sort_by(|a, b| b.cmp(a)); // use as stack
            let node = queue.pop().unwrap();
            sorted.push(py_str(&node));
            if let Some(neighbors) = adj.get(&node) {
                for n in neighbors {
                    if let Some(deg) = in_degree.get_mut(n) {
                        *deg -= 1;
                        if *deg == 0 {
                            queue.push(n.clone());
                        }
                    }
                }
            }
        }

        // If any nodes remain unprocessed, there's a cycle — just return empty
        // (stub behavior)
        if sorted.len() != nodes.len() {
            sorted.clear();
        }

        Ok(py_list(sorted))
    });

    d
}

// ---- pickle helper functions ----

/// Serialize a Python object to bytes using a simple custom format.
///
/// Format (byte markers):
///   N       -> None
///   T       -> True
///   F       -> False
///   I<val>\n -> int (decimal, newline-terminated)
///   G<val>\n -> float (decimal, newline-terminated)
///   S<len>:<utf8>  -> str (length-prefixed UTF-8)
///   B<len>:<bytes>  -> bytes (length-prefixed raw bytes)
///   [ ... ] -> list (elements serialized recursively)
///   ( ... ) -> tuple (elements serialized recursively)
///   { ... } -> dict (alternating key-value pairs serialized recursively)
fn pickle_serialize(obj: &PyObjectRef, buf: &mut Vec<u8>) -> PyResult<()> {
    match &*obj.borrow() {
        PyObject::None => buf.push(b'N'),
        PyObject::Bool(true) => buf.push(b'T'),
        PyObject::Bool(false) => buf.push(b'F'),
        PyObject::Int(n) => {
            buf.push(b'I');
            buf.extend_from_slice(n.to_string().as_bytes());
            buf.push(b'\n');
        }
        PyObject::Float(f) => {
            buf.push(b'G');
            let s = if f.is_nan() {
                "nan".to_string()
            } else if f.is_infinite() && f.is_sign_positive() {
                "inf".to_string()
            } else if f.is_infinite() {
                "-inf".to_string()
            } else {
                let s = format!("{:.17}", f);
                let s = s.trim_end_matches('0').to_string();
                if s.ends_with('.') {
                    format!("{}0", s)
                } else {
                    s
                }
            };
            buf.extend_from_slice(s.as_bytes());
            buf.push(b'\n');
        }
        PyObject::Str(s) => {
            buf.push(b'S');
            let bytes = s.as_bytes();
            buf.extend_from_slice(bytes.len().to_string().as_bytes());
            buf.push(b':');
            buf.extend_from_slice(bytes);
        }
        PyObject::Bytes(b) => {
            buf.push(b'B');
            buf.extend_from_slice(b.len().to_string().as_bytes());
            buf.push(b':');
            buf.extend_from_slice(b);
        }
        PyObject::List(items) => {
            buf.push(b'[');
            for item in items {
                pickle_serialize(item, buf)?;
            }
            buf.push(b']');
        }
        PyObject::Tuple(items) => {
            buf.push(b'(');
            for item in items {
                pickle_serialize(item, buf)?;
            }
            buf.push(b')');
        }
        PyObject::Dict(d) => {
            buf.push(b'{');
            for (k, v) in d.items() {
                pickle_serialize(&k, buf)?;
                pickle_serialize(&v, buf)?;
            }
            buf.push(b'}');
        }
        _ => {
            return Err(PyError::type_error(format!(
                "cannot pickle {} object",
                obj.borrow().type_name()
            )));
        }
    }
    Ok(())
}

/// Deserialize a Python object from bytes using the custom pickle format.
fn pickle_deserialize(data: &[u8], pos: &mut usize) -> PyResult<PyObjectRef> {
    if *pos >= data.len() {
        return Err(PyError::type_error("unexpected end of pickle data"));
    }
    let marker = data[*pos];
    *pos += 1;
    match marker {
        b'N' => Ok(py_none()),
        b'T' => Ok(py_bool(true)),
        b'F' => Ok(py_bool(false)),
        b'I' => {
            let start = *pos;
            while *pos < data.len() && data[*pos] != b'\n' {
                *pos += 1;
            }
            if *pos >= data.len() {
                return Err(PyError::type_error("unterminated integer in pickle data"));
            }
            let s = std::str::from_utf8(&data[start..*pos])
                .map_err(|_| PyError::type_error("invalid utf-8 in pickle int"))?;
            *pos += 1; // skip '\n'
            let n: num_bigint::BigInt = s
                .parse()
                .map_err(|_| PyError::type_error(format!("invalid integer: {}", s)))?;
            Ok(py_int(n))
        }
        b'G' => {
            let start = *pos;
            while *pos < data.len() && data[*pos] != b'\n' {
                *pos += 1;
            }
            if *pos >= data.len() {
                return Err(PyError::type_error("unterminated float in pickle data"));
            }
            let s = std::str::from_utf8(&data[start..*pos])
                .map_err(|_| PyError::type_error("invalid utf-8 in pickle float"))?;
            *pos += 1; // skip '\n'
            let f: f64 = s
                .parse()
                .map_err(|_| PyError::type_error(format!("invalid float: {}", s)))?;
            Ok(py_float(f))
        }
        b'S' => {
            let start = *pos;
            while *pos < data.len() && data[*pos] != b':' {
                *pos += 1;
            }
            if *pos >= data.len() {
                return Err(PyError::type_error(
                    "unterminated string length in pickle data",
                ));
            }
            let len_str = std::str::from_utf8(&data[start..*pos])
                .map_err(|_| PyError::type_error("invalid utf-8 in pickle string length"))?;
            let len: usize = len_str
                .parse()
                .map_err(|_| PyError::type_error(format!("invalid string length: {}", len_str)))?;
            *pos += 1; // skip ':'
            if *pos + len > data.len() {
                return Err(PyError::type_error("unexpected end of pickle string data"));
            }
            let s = std::str::from_utf8(&data[*pos..*pos + len])
                .map_err(|_| PyError::type_error("invalid utf-8 in pickle string"))?;
            *pos += len;
            Ok(py_str(s))
        }
        b'B' => {
            let start = *pos;
            while *pos < data.len() && data[*pos] != b':' {
                *pos += 1;
            }
            if *pos >= data.len() {
                return Err(PyError::type_error(
                    "unterminated bytes length in pickle data",
                ));
            }
            let len_str = std::str::from_utf8(&data[start..*pos])
                .map_err(|_| PyError::type_error("invalid utf-8 in pickle bytes length"))?;
            let len: usize = len_str
                .parse()
                .map_err(|_| PyError::type_error(format!("invalid bytes length: {}", len_str)))?;
            *pos += 1; // skip ':'
            if *pos + len > data.len() {
                return Err(PyError::type_error("unexpected end of pickle bytes data"));
            }
            let bytes = data[*pos..*pos + len].to_vec();
            *pos += len;
            Ok(PyObjectRef::imm(PyObject::Bytes(bytes)))
        }
        b'[' => {
            let mut items = Vec::new();
            while *pos < data.len() && data[*pos] != b']' {
                items.push(pickle_deserialize(data, pos)?);
            }
            if *pos >= data.len() {
                return Err(PyError::type_error("unterminated list in pickle data"));
            }
            *pos += 1; // skip ']'
            Ok(py_list(items))
        }
        b'(' => {
            let mut items = Vec::new();
            while *pos < data.len() && data[*pos] != b')' {
                items.push(pickle_deserialize(data, pos)?);
            }
            if *pos >= data.len() {
                return Err(PyError::type_error("unterminated tuple in pickle data"));
            }
            *pos += 1; // skip ')'
            Ok(py_tuple(items))
        }
        b'{' => {
            let mut dict = crate::object::PyDict::new();
            while *pos < data.len() && data[*pos] != b'}' {
                let key = pickle_deserialize(data, pos)?;
                if *pos >= data.len() {
                    return Err(PyError::type_error("unterminated dict in pickle data"));
                }
                let value = pickle_deserialize(data, pos)?;
                dict.set(key, value)?;
            }
            if *pos >= data.len() {
                return Err(PyError::type_error("unterminated dict in pickle data"));
            }
            *pos += 1; // skip '}'
            Ok(PyObjectRef::new(PyObject::Dict(dict)))
        }
        _ => Err(PyError::type_error(format!(
            "unknown pickle marker byte: 0x{:02x}",
            marker
        ))),
    }
}

pub fn create_pickle_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! pickle_func {
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

    pickle_func!("dumps", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("dumps() missing required argument"));
        }
        let mut buf = Vec::new();
        pickle_serialize(&args[0], &mut buf)?;
        Ok(PyObjectRef::imm(PyObject::Bytes(buf)))
    });

    pickle_func!("loads", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("loads() missing required argument"));
        }
        let data: Vec<u8> = match &*args[0].borrow() {
            PyObject::Bytes(b) => b.clone(),
            PyObject::Str(s) => s.as_bytes().to_vec(),
            _ => {
                return Err(PyError::type_error(
                    "loads() argument must be bytes or string",
                ))
            }
        };
        let mut pos = 0;
        let result = pickle_deserialize(&data, &mut pos)?;
        // Check there's no trailing garbage (except for flat values where pos may be at end)
        if pos != data.len() {
            return Err(PyError::type_error(format!(
                "pickle data has trailing bytes after value (pos={}, len={})",
                pos,
                data.len()
            )));
        }
        Ok(result)
    });

    d
}

pub fn create_logging_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! log_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
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
        let name = if args.is_empty() { "root".to_string() } else { args[0].str() };
        // Check registry first
        let cached = LOGGER_REGISTRY.with(|reg| reg.borrow().get(&name).cloned());
        if let Some(logger) = cached {
            return Ok(logger);
        }
        // Create a new Logger type
        let logger_typ = PyObjectRef::new(PyObject::Type {
            name: "Logger".to_string(),
            dict: {
                let mut type_dict = HashMap::new();
                type_dict.insert("info".to_string(), PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "info".to_string(),
                    func: logging_info,
                    self_obj: py_none(),
                }));
                type_dict.insert("debug".to_string(), PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "debug".to_string(),
                    func: logging_debug,
                    self_obj: py_none(),
                }));
                type_dict.insert("warning".to_string(), PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "warning".to_string(),
                    func: logging_warning,
                    self_obj: py_none(),
                }));
                type_dict.insert("error".to_string(), PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "error".to_string(),
                    func: logging_error,
                    self_obj: py_none(),
                }));
                type_dict
            },
            bases: vec![],
            mro: vec![],
        });
        let instance = PyObjectRef::new(PyObject::Instance {
            typ: logger_typ,
            dict: HashMap::from([
                ("name".to_string(), py_str(&name)),
            ]),
        });
        LOGGER_REGISTRY.with(|reg| reg.borrow_mut().insert(name.clone(), instance.clone()));
        Ok(instance)
    });

    // NullHandler class (needed by urllib3 and other libs)
    d.insert("NullHandler".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "NullHandler".to_string(),
        func: |_| {
            Ok(create_module("NullHandler", HashMap::from([
                ("emit".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
                    name: "emit".to_string(),
                    func: |_| Ok(py_none()),
                })),
                ("handle".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
                    name: "handle".to_string(),
                    func: |_| Ok(py_none()),
                })),
                ("setLevel".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
                    name: "setLevel".to_string(),
                    func: |_| Ok(py_none()),
                })),
            ])))
        },
    }));

    // Add level constants
    d.insert("CRITICAL".to_string(), py_int(50));
    d.insert("ERROR".to_string(), py_int(40));
    d.insert("WARNING".to_string(), py_int(30));
    d.insert("INFO".to_string(), py_int(20));
    d.insert("DEBUG".to_string(), py_int(10));
    d.insert("NOTSET".to_string(), py_int(0));

    d
}

thread_local! {
    static EXIT_CALLBACKS: std::cell::RefCell<Vec<PyObjectRef>> = std::cell::RefCell::new(Vec::new());
}

pub fn create_atexit_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    d.insert("register".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "register".to_string(),
        func: |args| {
            if args.is_empty() { return Err(PyError::type_error("register() requires a callable argument")); }
            EXIT_CALLBACKS.with(|cb| cb.borrow_mut().push(args[0].clone()));
            Ok(py_none())
        },
    }));
    d.insert("unregister".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "unregister".to_string(),
        func: |args| {
            if args.is_empty() { return Err(PyError::type_error("unregister() requires a callable argument")); }
            // Remove all occurrences of the given callable
            EXIT_CALLBACKS.with(|cb| {
                let mut callbacks = cb.borrow_mut();
                callbacks.retain(|c| !std::ptr::eq(c, &args[0]));
            });
            Ok(py_none())
        },
    }));
    d.insert("__name__".to_string(), py_str("atexit"));
    d
}

pub fn create_timeit_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! timeit_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    timeit_func!("timeit", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("timeit() missing required argument (stmt)"));
        }
        let stmt = args[0].str();
        let number: u64 = if args.len() > 1 {
            args[1].as_i64().unwrap_or(1_000_000) as u64
        } else {
            1_000_000
        };

        // Compile the statement
        let mut parser = crate::parser::Parser::new(&stmt);
        let program = parser.parse_program()
            .map_err(|e| PyError::type_error(format!("timeit parse error: {}", e)))?;
        let mut compiler = crate::compiler::Compiler::new();
        let code = compiler.compile(&program, "<timeit>")
            .map_err(|e| PyError::type_error(format!("timeit compile error: {}", e)))?;

        // Execute number times, measuring elapsed wall time
        let start = std::time::Instant::now();
        for _ in 0..number {
            let mut vm = crate::vm::VirtualMachine::new();
            vm.run(code.clone())
                .map_err(|e| PyError::type_error(format!("timeit error: {}", e)))?;
        }
        let elapsed = start.elapsed();
        let total_secs = elapsed.as_secs_f64();
        let per_loop = total_secs / number as f64;

        // Return the total time in seconds (as a float)
        Ok(py_float(per_loop))
    });

    // Also provide a repeat function for convenience
    timeit_func!("repeat", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("repeat() missing required argument (stmt)"));
        }
        let stmt = args[0].str();
        let repeat: u64 = if args.len() > 1 {
            args[1].as_i64().unwrap_or(3) as u64
        } else {
            3
        };
        let number: u64 = if args.len() > 2 {
            args[2].as_i64().unwrap_or(1_000_000) as u64
        } else {
            1_000_000
        };

        let mut parser = crate::parser::Parser::new(&stmt);
        let program = parser.parse_program()
            .map_err(|e| PyError::type_error(format!("timeit repeat parse error: {}", e)))?;
        let mut compiler = crate::compiler::Compiler::new();
        let code = compiler.compile(&program, "<timeit>")
            .map_err(|e| PyError::type_error(format!("timeit repeat compile error: {}", e)))?;

        let mut times = Vec::new();
        for _ in 0..repeat {
            let start = std::time::Instant::now();
            for _ in 0..number {
                let mut vm = crate::vm::VirtualMachine::new();
                vm.run(code.clone())
                    .map_err(|e| PyError::type_error(format!("timeit repeat error: {}", e)))?;
            }
            let elapsed = start.elapsed();
            times.push(py_float(elapsed.as_secs_f64()));
        }

        Ok(py_list(times))
    });

    // Default number of repetitions
    d.insert("default_number".to_string(), py_int(1_000_000));
    d.insert("default_repeat".to_string(), py_int(3));

    d
}

pub fn create_json_tool_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! jt_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    jt_func!("main", |_args| {
        // Read all of stdin
        let mut input = String::new();
        use std::io::Read;
        match std::io::stdin().read_to_string(&mut input) {
            Ok(_) => {
                // Parse JSON
                let parsed = json_decode(&input)?;
                // Pretty-print with indent=2
                let formatted = json_encode_full(&parsed, Some(2), true, 0)?;
                // Print to stdout
                println!("{}", formatted.str());
                Ok(py_none())
            }
            Err(e) => Err(PyError::runtime_error(format!("json.tool error reading stdin: {}", e))),
        }
    });

    d
}

pub fn create_cmath_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! cm_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }
    cm_func!("sqrt", |args| {
        if args.len() != 1 { return Err(PyError::type_error("sqrt() takes exactly one argument")); }
        let v = args[0].borrow();
        match &*v { PyObject::Int(i) => Ok(py_float(i.to_f64().unwrap_or(0.0).sqrt())), PyObject::Float(f) => Ok(py_float(f.sqrt())), _ => Err(PyError::type_error("sqrt() argument must be a number")) }
    });
    cm_func!("sin", |args| {
        if args.len() != 1 { return Err(PyError::type_error("sin() takes exactly one argument")); }
        let v = args[0].borrow();
        match &*v { PyObject::Int(i) => Ok(py_float(i.to_f64().unwrap_or(0.0).sin())), PyObject::Float(f) => Ok(py_float(f.sin())), _ => Err(PyError::type_error("sin() argument must be a number")) }
    });
    cm_func!("cos", |args| {
        if args.len() != 1 { return Err(PyError::type_error("cos() takes exactly one argument")); }
        let v = args[0].borrow();
        match &*v { PyObject::Int(i) => Ok(py_float(i.to_f64().unwrap_or(0.0).cos())), PyObject::Float(f) => Ok(py_float(f.cos())), _ => Err(PyError::type_error("cos() argument must be a number")) }
    });
    d
}

pub fn create_hashlib_extra_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! hle_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    hle_func!("md5", |args| {
        if args.len() != 1 { return Err(PyError::type_error("md5() takes exactly one argument")); }
        let data = args[0].borrow();
        let bytes = match &*data {
            PyObject::Bytes(b) => b.clone(),
            PyObject::Str(s) => s.as_bytes().to_vec(),
            _ => return Err(PyError::type_error("md5() argument must be bytes or str")),
        };
        use std::collections::hash_map::DefaultHasher;
        use std::hash::Hasher;
        let mut hasher = DefaultHasher::new();
        hasher.write(b"md5");
        hasher.write(&bytes);
        Ok(py_str(&format!("{:016x}", hasher.finish())))
    });

    hle_func!("sha1", |args| {
        if args.len() != 1 { return Err(PyError::type_error("sha1() takes exactly one argument")); }
        let data = args[0].borrow();
        let bytes = match &*data {
            PyObject::Bytes(b) => b.clone(),
            PyObject::Str(s) => s.as_bytes().to_vec(),
            _ => return Err(PyError::type_error("sha1() argument must be bytes or str")),
        };
        use std::collections::hash_map::DefaultHasher;
        use std::hash::Hasher;
        let mut hasher = DefaultHasher::new();
        hasher.write(b"sha1");
        hasher.write(&bytes);
        Ok(py_str(&format!("{:016x}", hasher.finish())))
    });

    hle_func!("sha256", |args| {
        if args.len() != 1 { return Err(PyError::type_error("sha256() takes exactly one argument")); }
        let data = args[0].borrow();
        let bytes = match &*data {
            PyObject::Bytes(b) => b.clone(),
            PyObject::Str(s) => s.as_bytes().to_vec(),
            _ => return Err(PyError::type_error("sha256() argument must be bytes or str")),
        };
        use std::collections::hash_map::DefaultHasher;
        use std::hash::Hasher;
        let mut hasher = DefaultHasher::new();
        hasher.write(b"sha256");
        hasher.write(&bytes);
        Ok(py_str(&format!("{:016x}", hasher.finish())))
    });

    d
}

pub fn create_queue_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! q_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    q_func!("Queue", |_args| {
        let inner = std::sync::Arc::new(std::sync::Mutex::new(QueueInner {
            queue: std::collections::VecDeque::new(),
        }));
        Ok(PyObjectRef::new(PyObject::Queue(inner)))
    });

    d
}

pub fn create_array_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();

    // Array type as a factory function
    d.insert("array".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "array".to_string(),
        func: |args| {
            if args.is_empty() {
                return Err(PyError::type_error("array() requires at least 1 argument (typecode)"));
            }
            let typecode_str = args[0].str();
            if typecode_str.is_empty() {
                return Err(PyError::value_error("empty typecode".to_string()));
            }
            let typecode = typecode_str.chars().next().unwrap();
            if typecode != 'i' && typecode != 'f' && typecode != 'd' {
                return Err(PyError::value_error(format!("bad typecode '{}'", typecode)));
            }
            let mut data: Vec<f64> = Vec::new();
            if args.len() > 1 {
                let init = &args[1];
                let init_borrowed = init.borrow();
                match &*init_borrowed {
                    PyObject::List(items) => {
                        for item in items {
                            if typecode == 'i' {
                                data.push(item.as_i64().unwrap_or(0) as f64);
                            } else {
                                data.push(item.as_f64().unwrap_or(0.0));
                            }
                        }
                    }
                    PyObject::Tuple(items) => {
                        for item in items {
                            if typecode == 'i' {
                                data.push(item.as_i64().unwrap_or(0) as f64);
                            } else {
                                data.push(item.as_f64().unwrap_or(0.0));
                            }
                        }
                    }
                    _ => {
                        // Try iterating
                        let iter_obj = builtin_iter(&[init.clone()])?;
                        loop {
                            match builtin_next(&[iter_obj.clone()]) {
                                Ok(item) => {
                                    if typecode == 'i' {
                                        data.push(item.as_i64().unwrap_or(0) as f64);
                                    } else {
                                        data.push(item.as_f64().unwrap_or(0.0));
                                    }
                                }
                                Err(PyError::StopIteration) => break,
                                Err(e) => return Err(e),
                            }
                        }
                    }
                }
            }
            Ok(PyObjectRef::new(PyObject::Array(PyArray { typecode, data })))
        },
    }));

    d
}

pub fn create_thread_module_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! thr_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    thr_func!("start_new_thread", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("start_new_thread() requires at least 2 arguments (function, args)"));
        }
        let func = args[0].clone();
        let func_args = if let PyObject::Tuple(items) = &*args[1].borrow() {
            items.clone()
        } else {
            return Err(PyError::type_error("start_new_thread() args must be a tuple"));
        };
        // Call function synchronously
        crate::object::call_function(&func, func_args)?;
        Ok(py_int(0))
    });

    thr_func!("allocate_lock", |_| {
        let inner = std::sync::Arc::new(std::sync::Mutex::new(LockInner {
            lock: std::sync::atomic::AtomicBool::new(false),
        }));
        Ok(PyObjectRef::new(PyObject::Lock(inner)))
    });

    d
}

pub fn create_signal_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();

    // Signal constants
    d.insert("SIGINT".to_string(), py_int(2));
    d.insert("SIGTERM".to_string(), py_int(15));
    d.insert("SIGHUP".to_string(), py_int(1));
    d.insert("SIGILL".to_string(), py_int(4));
    d.insert("SIGFPE".to_string(), py_int(8));
    d.insert("SIGKILL".to_string(), py_int(9));
    d.insert("SIGSEGV".to_string(), py_int(11));
    d.insert("SIGPIPE".to_string(), py_int(13));
    d.insert("SIGALRM".to_string(), py_int(14));
    d.insert("SIG_DFL".to_string(), py_int(0));
    d.insert("SIG_IGN".to_string(), py_int(1));

    macro_rules! sig_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    sig_func!("signal", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("signal() requires 2 arguments (signalnum, handler)"));
        }
        Ok(py_none())
    });

    sig_func!("getsignal", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("getsignal() missing required argument (signalnum)"));
        }
        Ok(py_int(0))
    });

    d
}

pub fn create_gc_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! gc_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    gc_func!("collect", |_| {
        Ok(py_int(0))
    });

    gc_func!("enable", |_| {
        Ok(py_none())
    });

    gc_func!("disable", |_| {
        Ok(py_none())
    });

    gc_func!("isenabled", |_| {
        Ok(py_bool(true))
    });

    d
}

pub fn create_sysconfig_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! syscfg_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    syscfg_func!("get_config_var", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("get_config_var() missing required argument (name)"));
        }
        Ok(py_none())
    });

    syscfg_func!("get_config_vars", |_| {
        Ok(py_dict())
    });

    syscfg_func!("get_platform", |_| {
        Ok(py_str("linux-x86_64"))
    });

    syscfg_func!("get_python_version", |_| {
        Ok(py_str("3.13"))
    });

    d
}

pub fn create_locale_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! loc_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    // LC_* constants matching CPython values
    d.insert("LC_ALL".to_string(), py_int(6i64));
    d.insert("LC_COLLATE".to_string(), py_int(3i64));
    d.insert("LC_CTYPE".to_string(), py_int(0i64));
    d.insert("LC_MONETARY".to_string(), py_int(4i64));
    d.insert("LC_NUMERIC".to_string(), py_int(1i64));
    d.insert("LC_TIME".to_string(), py_int(2i64));
    d.insert("LC_MESSAGES".to_string(), py_int(5i64));

    // getlocale() — returns (lang_code, encoding) tuple
    loc_func!("getlocale", |args| {
        let category = if args.len() >= 1 {
            args[0].as_i64().unwrap_or(6) // default LC_ALL
        } else {
            6
        };
        let _ = category; // unused in stub
        Ok(py_tuple(vec![
            py_str("en_US"),
            py_str("UTF-8"),
        ]))
    });

    // setlocale(category, locale) — stub that returns the locale string
    loc_func!("setlocale", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("setlocale() requires at least 2 arguments (category, locale)"));
        }
        let locale = args[1].str();
        // Attempt to set locale via system
        let _ = std::env::set_var("LANG", &locale);
        Ok(py_str(&locale))
    });

    // localeconv() — stub returning dict of locale conventions
    loc_func!("localeconv", |args| {
        let _ = args;
        let dict = py_dict();
        if let PyObject::Dict(d) = &mut *dict.borrow_mut() {
            d.set(py_str("decimal_point"), py_str(".")).ok();
            d.set(py_str("thousands_sep"), py_str(",")).ok();
            d.set(py_str("grouping"), py_list(vec![py_int(3), py_int(0)])).ok();
            d.set(py_str("currency_symbol"), py_str("$")).ok();
            d.set(py_str("mon_decimal_point"), py_str(".")).ok();
            d.set(py_str("mon_thousands_sep"), py_str(",")).ok();
            d.set(py_str("mon_grouping"), py_list(vec![py_int(3), py_int(0)])).ok();
            d.set(py_str("positive_sign"), py_str("")).ok();
            d.set(py_str("negative_sign"), py_str("-")).ok();
            d.set(py_str("int_frac_digits"), py_int(2)).ok();
            d.set(py_str("frac_digits"), py_int(2)).ok();
            d.set(py_str("p_cs_precedes"), py_int(1)).ok();
            d.set(py_str("n_cs_precedes"), py_int(1)).ok();
            d.set(py_str("p_sep_by_space"), py_int(0)).ok();
            d.set(py_str("n_sep_by_space"), py_int(0)).ok();
            d.set(py_str("p_sign_posn"), py_int(1)).ok();
            d.set(py_str("n_sign_posn"), py_int(1)).ok();
            d.set(py_str("int_curr_symbol"), py_str("USD ")).ok();
        }
        Ok(dict)
    });

    // getdefaultlocale() — returns (lang_code, encoding)
    loc_func!("getdefaultlocale", |_| {
        Ok(py_tuple(vec![
            py_str("en_US"),
            py_str("UTF-8"),
        ]))
    });

    // getpreferredencoding() — returns 'UTF-8'
    loc_func!("getpreferredencoding", |_| {
        Ok(py_str("UTF-8"))
    });

    // strcoll(a, b) — string comparison using locale
    loc_func!("strcoll", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("strcoll() requires 2 arguments (str1, str2)"));
        }
        let a = args[0].str();
        let b = args[1].str();
        Ok(py_int(a.cmp(&b) as i64))
    });

    // strxfrm(s) — string transformation for locale-aware comparison
    loc_func!("strxfrm", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("strxfrm() missing required argument (str)"));
        }
        Ok(py_str(&args[0].str()))
    });

    d
}

pub fn create_colorsys_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! cs_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    // Helper: clamp a float to [0.0, 1.0]
    fn clampf(v: f64) -> f64 {
        if v < 0.0 { 0.0 } else if v > 1.0 { 1.0 } else { v }
    }

    // one third = 1.0 / 3.0
    const ONE_THIRD: f64 = 1.0 / 3.0;
    const TWO_THIRD: f64 = 2.0 / 3.0;

    fn hue_to_rgb(m1: f64, m2: f64, mut h: f64) -> f64 {
        if h < 0.0 { h += 1.0; }
        if h > 1.0 { h -= 1.0; }
        if h * 6.0 < 1.0 { return m1 + (m2 - m1) * h * 6.0; }
        if h * 2.0 < 1.0 { return m2; }
        if h * 3.0 < 2.0 { return m1 + (m2 - m1) * (TWO_THIRD - h) * 6.0; }
        m1
    }

    cs_func!("rgb_to_hsv", |args| {
        if args.len() < 3 {
            return Err(PyError::type_error("rgb_to_hsv() requires 3 arguments (r, g, b)"));
        }
        let r = args[0].as_f64().ok_or_else(|| PyError::type_error("r must be a number"))?;
        let g = args[1].as_f64().ok_or_else(|| PyError::type_error("g must be a number"))?;
        let b = args[2].as_f64().ok_or_else(|| PyError::type_error("b must be a number"))?;

        let maxc = r.max(g).max(b);
        let minc = r.min(g).min(b);
        let v = maxc;
        if minc == maxc {
            return Ok(py_tuple(vec![py_float(0.0), py_float(0.0), py_float(v)]));
        }
        let s = (maxc - minc) / maxc;
        let rc = (maxc - r) / (maxc - minc);
        let gc = (maxc - g) / (maxc - minc);
        let bc = (maxc - b) / (maxc - minc);
        let h = if r == maxc {
            bc - gc
        } else if g == maxc {
            2.0 + rc - bc
        } else {
            4.0 + gc - rc
        };
        let h = (h / 6.0) % 1.0;
        let h = if h < 0.0 { h + 1.0 } else { h };
        Ok(py_tuple(vec![py_float(h), py_float(s), py_float(v)]))
    });

    cs_func!("hsv_to_rgb", |args| {
        if args.len() < 3 {
            return Err(PyError::type_error("hsv_to_rgb() requires 3 arguments (h, s, v)"));
        }
        let h = args[0].as_f64().ok_or_else(|| PyError::type_error("h must be a number"))?;
        let s = args[1].as_f64().ok_or_else(|| PyError::type_error("s must be a number"))?;
        let v = args[2].as_f64().ok_or_else(|| PyError::type_error("v must be a number"))?;

        if s == 0.0 {
            let gray = clampf(v);
            return Ok(py_tuple(vec![py_float(gray), py_float(gray), py_float(gray)]));
        }

        let h = (h % 1.0 + 1.0) % 1.0;
        let hi = (h * 6.0).floor() as i32;
        let f = h * 6.0 - hi as f64;
        let p = v * (1.0 - s);
        let q = v * (1.0 - s * f);
        let t = v * (1.0 - s * (1.0 - f));

        let (r, g, b) = match hi % 6 {
            0 => (v, t, p),
            1 => (q, v, p),
            2 => (p, v, t),
            3 => (p, q, v),
            4 => (t, p, v),
            _ => (v, p, q),
        };
        Ok(py_tuple(vec![py_float(clampf(r)), py_float(clampf(g)), py_float(clampf(b))]))
    });

    cs_func!("rgb_to_hls", |args| {
        if args.len() < 3 {
            return Err(PyError::type_error("rgb_to_hls() requires 3 arguments (r, g, b)"));
        }
        let r = args[0].as_f64().ok_or_else(|| PyError::type_error("r must be a number"))?;
        let g = args[1].as_f64().ok_or_else(|| PyError::type_error("g must be a number"))?;
        let b = args[2].as_f64().ok_or_else(|| PyError::type_error("b must be a number"))?;

        let maxc = r.max(g).max(b);
        let minc = r.min(g).min(b);
        let l = (minc + maxc) / 2.0;
        if minc == maxc {
            return Ok(py_tuple(vec![py_float(0.0), py_float(l), py_float(0.0)]));
        }
        let s = if l <= 0.5 {
            (maxc - minc) / (maxc + minc)
        } else {
            (maxc - minc) / (2.0 - maxc - minc)
        };
        let rc = (maxc - r) / (maxc - minc);
        let gc = (maxc - g) / (maxc - minc);
        let bc = (maxc - b) / (maxc - minc);
        let h = if r == maxc {
            bc - gc
        } else if g == maxc {
            2.0 + rc - bc
        } else {
            4.0 + gc - rc
        };
        let h = (h / 6.0) % 1.0;
        let h = if h < 0.0 { h + 1.0 } else { h };
        Ok(py_tuple(vec![py_float(h), py_float(l), py_float(s)]))
    });

    cs_func!("hls_to_rgb", |args| {
        if args.len() < 3 {
            return Err(PyError::type_error("hls_to_rgb() requires 3 arguments (h, l, s)"));
        }
        let h = args[0].as_f64().ok_or_else(|| PyError::type_error("h must be a number"))?;
        let l = args[1].as_f64().ok_or_else(|| PyError::type_error("l must be a number"))?;
        let s = args[2].as_f64().ok_or_else(|| PyError::type_error("s must be a number"))?;

        if s == 0.0 {
            return Ok(py_tuple(vec![py_float(l), py_float(l), py_float(l)]));
        }
        let m2 = if l <= 0.5 { l * (1.0 + s) } else { l + s - l * s };
        let m1 = 2.0 * l - m2;
        let r = hue_to_rgb(m1, m2, h + ONE_THIRD);
        let g = hue_to_rgb(m1, m2, h);
        let b = hue_to_rgb(m1, m2, h - ONE_THIRD);
        Ok(py_tuple(vec![py_float(clampf(r)), py_float(clampf(g)), py_float(clampf(b))]))
    });

    d
}

pub fn create_wave_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();

    fn read_wave_params(data: &[u8]) -> Result<(i32, i32, i32, i32, usize), String> {
        if data.len() < 44 {
            return Err("Not a valid WAV file: too short".to_string());
        }
        if &data[0..4] != b"RIFF" || &data[8..12] != b"WAVE" {
            return Err("Not a valid WAV file: missing RIFF/WAVE header".to_string());
        }
        // Find fmt chunk — skip RIFF header (12 bytes)
        let mut offset = 12usize;
        let (fmt_offset, fmt_size) = loop {
            if offset + 8 > data.len() {
                return Err("Not a valid WAV file: no fmt chunk found".to_string());
            }
            let chunk_id = &data[offset..offset+4];
            let chunk_size = u32::from_le_bytes([
                data[offset+4], data[offset+5], data[offset+6], data[offset+7],
            ]) as usize;
            if chunk_id == b"fmt " {
                break (offset, chunk_size);
            }
            offset += 8 + chunk_size;
            if offset % 2 != 0 { offset += 1; } // pad to word boundary
            if offset >= data.len() {
                return Err("Not a valid WAV file: no fmt chunk found".to_string());
            }
        };

        let fmt_data = &data[fmt_offset..];
        if fmt_data.len() < 24 {
            return Err("Not a valid WAV file: fmt chunk too small".to_string());
        }

        let audio_format = u16::from_le_bytes([fmt_data[8], fmt_data[9]]);
        if audio_format != 1 {
            return Err(format!("Unsupported WAV audio format: {} (only PCM/1 supported)", audio_format));
        }
        let nchannels = u16::from_le_bytes([fmt_data[10], fmt_data[11]]) as i32;
        let framerate = i32::from_le_bytes([fmt_data[12], fmt_data[13], fmt_data[14], fmt_data[15]]);
        // Byte rate is at [16..20], block align at [20..22]
        let bits_per_sample = u16::from_le_bytes([fmt_data[22], fmt_data[23]]);
        let sampwidth = (bits_per_sample / 8) as i32;
        if sampwidth == 0 {
            return Err("Invalid sample width: 0 bytes per sample".to_string());
        }

        // Find data chunk
        let mut data_offset = fmt_offset + 8 + fmt_size;
        if data_offset % 2 != 0 { data_offset += 1; }

        let (data_chunk_start, data_size) = loop {
            if data_offset + 8 > data.len() {
                return Err("Not a valid WAV file: no data chunk found".to_string());
            }
            let chunk_id = &data[data_offset..data_offset+4];
            let chunk_size = u32::from_le_bytes([
                data[data_offset+4], data[data_offset+5], data[data_offset+6], data[data_offset+7],
            ]) as usize;
            if chunk_id == b"data" {
                break (data_offset + 8, chunk_size);
            }
            data_offset += 8 + chunk_size;
            if data_offset % 2 != 0 { data_offset += 1; }
            if data_offset >= data.len() {
                return Err("Not a valid WAV file: no data chunk found".to_string());
            }
        };

        let nframes = if sampwidth > 0 && nchannels > 0 {
            (data_size as i32) / (sampwidth * nchannels)
        } else {
            0
        };

        Ok((nchannels, sampwidth, framerate, nframes, data_chunk_start))
    }

    // Wave_read module-level alias — direct instantiation not allowed
    d.insert("Wave_read".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "Wave_read".to_string(),
        func: |_args| {
            Err(PyError::type_error("Wave_read cannot be instantiated directly; use wave.open()"))
        },
    }));

    d.insert("open".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "open".to_string(),
        func: |args| {
            if args.is_empty() {
                return Err(PyError::type_error("open() missing required argument: file"));
            }
            let file_path = args[0].str();
            let mode = if args.len() > 1 { args[1].str() } else { "r".to_string() };
            let mode = mode.trim();
            if mode != "r" && mode != "rb" {
                return Err(PyError::type_error(format!("wave.open() only supports mode='r' or 'rb', got '{}'", mode)));
            }

            let data = match std::fs::read(&file_path) {
                Ok(d) => d,
                Err(e) => return Err(PyError::type_error(format!("Cannot open wave file: {}", e))),
            };

            match read_wave_params(&data) {
                Ok((nchannels, sampwidth, framerate, nframes, data_start)) => {
                    // Build a proper Type with methods so args[0] is self
                    let mut type_dict = HashMap::new();

                    type_dict.insert("getparams".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
                        name: "getparams".to_string(),
                        func: |gp_args| {
                            if gp_args.is_empty() {
                                return Err(PyError::type_error("getparams() missing self argument"));
                            }
                            let inst = gp_args[0].borrow();
                            if let PyObject::Instance { dict, .. } = &*inst {
                                let nc = dict.get("nchannels").and_then(|v| v.as_i64()).unwrap_or(0);
                                let sw = dict.get("sampwidth").and_then(|v| v.as_i64()).unwrap_or(0);
                                let fr = dict.get("framerate").and_then(|v| v.as_i64()).unwrap_or(0);
                                let nf = dict.get("nframes").and_then(|v| v.as_i64()).unwrap_or(0);
                                Ok(py_tuple(vec![
                                    py_int(nc),
                                    py_int(sw),
                                    py_int(fr),
                                    py_int(nf),
                                    py_str("NONE"),
                                    py_str("not compressed"),
                                ]))
                            } else {
                                Err(PyError::type_error("getparams: not a Wave_read instance"))
                            }
                        },
                    }));

                    type_dict.insert("readframes".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
                        name: "readframes".to_string(),
                        func: |rf_args| {
                            if rf_args.is_empty() {
                                return Err(PyError::type_error("readframes() missing required argument: self"));
                            }
                            let n = if rf_args.len() > 1 {
                                rf_args[1].as_i64().ok_or_else(|| PyError::type_error("readframes() argument must be an integer"))? as usize
                            } else {
                                0
                            };
                            if n == 0 {
                                return Ok(PyObjectRef::imm(PyObject::Bytes(vec![])));
                            }
                            // Read nchannels, sampwidth, _data, _data_start from instance dict
                            let (nc_r, sw_r, dc_opt, ds_r) = {
                                let inst = rf_args[0].borrow();
                                if let PyObject::Instance { dict, .. } = &*inst {
                                    let nc_r = dict.get("nchannels").and_then(|v| v.as_i64()).unwrap_or(0) as usize;
                                    let sw_r = dict.get("sampwidth").and_then(|v| v.as_i64()).unwrap_or(0) as usize;
                                    let dc_opt = dict.get("_data").cloned();
                                    let ds_r = dict.get("_data_start").and_then(|v| v.as_i64()).unwrap_or(0) as usize;
                                    (nc_r, sw_r, dc_opt, ds_r)
                                } else {
                                    return Err(PyError::type_error("readframes: not a Wave_read instance"));
                                }
                            };
                            let frame_size = sw_r * nc_r;
                            if frame_size == 0 {
                                return Ok(PyObjectRef::imm(PyObject::Bytes(vec![])));
                            }
                            let dc = match dc_opt {
                                Some(d) => {
                                    let b = d.borrow();
                                    if let PyObject::Bytes(byte_data) = &*b {
                                        byte_data.clone()
                                    } else { vec![] }
                                }
                                None => vec![],
                            };
                            let nframes_avail = dc.len().saturating_sub(ds_r) / frame_size;
                            let n_to_read = n.min(nframes_avail);
                            let end = ds_r + n_to_read * frame_size;
                            if end > dc.len() || end <= ds_r {
                                return Ok(PyObjectRef::imm(PyObject::Bytes(vec![])));
                            }
                            let frame_data = dc[ds_r..end].to_vec();
                            Ok(PyObjectRef::imm(PyObject::Bytes(frame_data)))
                        },
                    }));

                    type_dict.insert("close".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
                        name: "close".to_string(),
                        func: |_| Ok(py_none()),
                    }));

                    let typ = PyObjectRef::new(PyObject::Type {
                        name: "Wave_read".to_string(),
                        dict: type_dict,
                        bases: vec![],
                        mro: vec![],
                    });

                    let mut instance_dict = HashMap::new();
                    instance_dict.insert("nchannels".to_string(), py_int(nchannels as i64));
                    instance_dict.insert("sampwidth".to_string(), py_int(sampwidth as i64));
                    instance_dict.insert("framerate".to_string(), py_int(framerate as i64));
                    instance_dict.insert("nframes".to_string(), py_int(nframes as i64));
                    instance_dict.insert("comptype".to_string(), py_str("NONE"));
                    instance_dict.insert("compname".to_string(), py_str("not compressed"));
                    instance_dict.insert("_data".to_string(), PyObjectRef::imm(PyObject::Bytes(data.clone())));
                    instance_dict.insert("_data_start".to_string(), py_int(data_start as i64));

                    Ok(PyObjectRef::new(PyObject::Instance {
                        typ,
                        dict: instance_dict,
                    }))
                }
                Err(e) => Err(PyError::type_error(e)),
            }
        },
    }));

    d
}

// ---- email module ----

fn email_message_getitem(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 2 {
        return Err(PyError::type_error("__getitem__() takes at least 2 arguments (self, key)"));
    }
    let key = args[1].str();
    let inst = args[0].borrow();
    if let PyObject::Instance { dict, .. } = &*inst {
        let header_key = format!("_header_{}", key);
        match dict.get(&header_key) {
            Some(val) => Ok(val.clone()),
            None => Ok(py_none()),
        }
    } else {
        Err(PyError::type_error("EmailMessage instance required"))
    }
}

fn email_message_setitem(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 3 {
        return Err(PyError::type_error("__setitem__() takes at least 3 arguments (self, key, value)"));
    }
    let key = args[1].str();
    let value = args[2].str();
    let mut inst = args[0].borrow_mut();
    if let PyObject::Instance { dict, .. } = &mut *inst {
        let header_key = format!("_header_{}", key);
        dict.insert(header_key, py_str(&value));
    }
    Ok(py_none())
}

fn email_message_set_content(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 2 {
        return Err(PyError::type_error("set_content() takes at least 1 argument (text)"));
    }
    let text = args[1].str();
    let mut inst = args[0].borrow_mut();
    if let PyObject::Instance { dict, .. } = &mut *inst {
        dict.insert("_content".to_string(), py_str(&text));
        dict.insert("_content_type".to_string(), py_str("text/plain"));
    }
    Ok(py_none())
}

fn email_message_as_string(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error("as_string() takes at least 1 argument (self)"));
    }
    let inst = args[0].borrow();
    if let PyObject::Instance { dict, .. } = &*inst {
        // Collect headers
        let mut headers: Vec<(String, String)> = Vec::new();
        for (k, v) in dict.iter() {
            if let Some(header_name) = k.strip_prefix("_header_") {
                headers.push((header_name.to_string(), v.str()));
            }
        }
        // Sort known headers first for readability
        let priority = |name: &str| -> usize {
            match name {
                "From" => 0,
                "To" => 1,
                "Subject" => 2,
                _ => 3,
            }
        };
        headers.sort_by_key(|(k, _)| priority(k));

        let content = dict.get("_content").map(|v| v.str()).unwrap_or_default();

        let mut result = String::new();
        for (name, value) in &headers {
            result.push_str(&format!("{}: {}\r\n", name, value));
        }
        result.push_str("\r\n");
        result.push_str(&content);

        Ok(py_str(&result))
    } else {
        Err(PyError::type_error("EmailMessage instance required"))
    }
}

fn email_message_repr(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error("__repr__() takes at least 1 argument (self)"));
    }
    let inst = args[0].borrow();
    if let PyObject::Instance { dict, .. } = &*inst {
        let subject = dict.get("_header_Subject").map(|v| v.str()).unwrap_or_default();
        let from_addr = dict.get("_header_From").map(|v| v.str()).unwrap_or_default();
        let to_addr = dict.get("_header_To").map(|v| v.str()).unwrap_or_default();
        Ok(py_str(&format!("<EmailMessage: From: {}, To: {}, Subject: {}>", from_addr, to_addr, subject)))
    } else {
        Err(PyError::type_error("EmailMessage instance required"))
    }
}

fn email_message_constructor(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    // Create the EmailMessage type
    let mut type_dict = HashMap::new();
    type_dict.insert("__getitem__".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "__getitem__".to_string(),
        func: email_message_getitem,
    }));
    type_dict.insert("__setitem__".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "__setitem__".to_string(),
        func: email_message_setitem,
    }));
    type_dict.insert("__repr__".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "__repr__".to_string(),
        func: email_message_repr,
    }));
    type_dict.insert("set_content".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "set_content".to_string(),
        func: email_message_set_content,
    }));
    type_dict.insert("as_string".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "as_string".to_string(),
        func: email_message_as_string,
    }));

    let email_type = PyObjectRef::new(PyObject::Type {
        name: "EmailMessage".to_string(),
        dict: type_dict,
        bases: vec![],
        mro: vec![],
    });

    // Create instance with empty dict
    let instance = PyObjectRef::new(PyObject::Instance {
        typ: email_type,
        dict: HashMap::new(),
    });

    Ok(instance)
}

pub fn create_email_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! email_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    // EmailMessage class constructor (callable)
    d.insert("EmailMessage".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "EmailMessage".to_string(),
        func: email_message_constructor,
    }));

    // MIMEText is in email.mime.text, but we provide a stub here for convenience
    email_func!("MIMEText", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("MIMEText() missing required argument"));
        }
        let body = args[0].str();
        let subtype = if args.len() > 1 { args[1].str() } else { "plain".to_string() };

        // Create a simple MIMEText instance (EmailMessage-like)
        let mut type_dict = HashMap::new();
        type_dict.insert("as_string".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
            name: "as_string".to_string(),
            func: |a| {
                let inst = a[0].borrow();
                if let PyObject::Instance { dict, .. } = &*inst {
                    let content = dict.get("_content").map(|v| v.str()).unwrap_or_default();
                    let ct = dict.get("_content_type").map(|v| v.str()).unwrap_or_default();
                    let mut result = format!("Content-Type: {}\r\n", ct);
                    result.push_str(&format!("Content-Transfer-Encoding: 7bit\r\n"));
                    result.push_str("\r\n");
                    result.push_str(&content);
                    Ok(py_str(&result))
                } else {
                    Err(PyError::type_error("MIMEText instance required"))
                }
            },
        }));

        let mime_type = PyObjectRef::new(PyObject::Type {
            name: "MIMEText".to_string(),
            dict: type_dict,
            bases: vec![],
            mro: vec![],
        });

        let mut instance_dict = HashMap::new();
        instance_dict.insert("_content".to_string(), py_str(&body));
        instance_dict.insert("_content_type".to_string(), py_str(&format!("text/{}", subtype)));

        Ok(PyObjectRef::new(PyObject::Instance {
            typ: mime_type,
            dict: instance_dict,
        }))
    });

    d
}

pub fn create_email_mime_text_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    d.insert("MIMEText".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "MIMEText".to_string(),
        func: |args| {
            if args.is_empty() {
                return Err(PyError::type_error("MIMEText() missing required argument"));
            }
            let body = args[0].str();
            let subtype = if args.len() > 1 { args[1].str() } else { "plain".to_string() };

            let mut type_dict = HashMap::new();
            type_dict.insert("as_string".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
                name: "as_string".to_string(),
                func: |a| {
                    let inst = a[0].borrow();
                    if let PyObject::Instance { dict, .. } = &*inst {
                        let content = dict.get("_content").map(|v| v.str()).unwrap_or_default();
                        let ct = dict.get("_content_type").map(|v| v.str()).unwrap_or_default();
                        let mut result = format!("Content-Type: {}\r\n", ct);
                        result.push_str("Content-Transfer-Encoding: 7bit\r\n");
                        result.push_str("\r\n");
                        result.push_str(&content);
                        Ok(py_str(&result))
                    } else {
                        Err(PyError::type_error("MIMEText instance required"))
                    }
                },
            }));

            let mime_type = PyObjectRef::new(PyObject::Type {
                name: "MIMEText".to_string(),
                dict: type_dict,
                bases: vec![],
                mro: vec![],
            });

            let mut instance_dict = HashMap::new();
            instance_dict.insert("_content".to_string(), py_str(&body));
            instance_dict.insert("_content_type".to_string(), py_str(&format!("text/{}", subtype)));

            Ok(PyObjectRef::new(PyObject::Instance {
                typ: mime_type,
                dict: instance_dict,
            }))
        },
    }));
    d
}

pub fn create_configparser_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();

    // Helper: parse INI string into sections
    fn parse_ini_string(data: &str) -> HashMap<String, HashMap<String, String>> {
        let mut sections: HashMap<String, HashMap<String, String>> = HashMap::new();
        let mut current_section: Option<String> = None;

        // Start with a pseudo-section for DEFAULT values
        sections.insert("DEFAULT".to_string(), HashMap::new());

        for line in data.lines() {
            let trimmed = line.trim();
            // Skip empty lines and comments
            if trimmed.is_empty() || trimmed.starts_with(';') || trimmed.starts_with('#') {
                continue;
            }

            // Section header: [sectionname]
            if trimmed.starts_with('[') {
                if let Some(end) = trimmed.find(']') {
                    let name = trimmed[1..end].trim().to_string();
                    if !name.is_empty() {
                        current_section = Some(name.clone());
                        sections.entry(name).or_insert_with(HashMap::new);
                    }
                }
                continue;
            }

            // Key = value (or key: value)
            if let Some(eq_pos) = trimmed.find('=').or_else(|| trimmed.find(':')) {
                let key = trimmed[..eq_pos].trim().to_string();
                let value = trimmed[eq_pos + 1..].trim().to_string();
                if !key.is_empty() {
                    let section_name = current_section.clone().unwrap_or_else(|| "DEFAULT".to_string());
                    let section = sections.entry(section_name).or_insert_with(HashMap::new);
                    section.insert(key, value);
                }
            }
        }

        sections
    }

    // ConfigParser class — constructor
    d.insert("ConfigParser".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "ConfigParser".to_string(),
        func: |args| {
            let mut type_dict = HashMap::new();

            // read_string(self, string) — parse INI from a string
            type_dict.insert("read_string".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
                name: "read_string".to_string(),
                func: |inner_args| {
                    if inner_args.len() < 2 {
                        return Err(PyError::type_error("read_string() missing required argument: string"));
                    }
                    let data = inner_args[1].str();

                    let sections_ref = {
                        let inst = inner_args[0].borrow();
                        if let PyObject::Instance { dict, .. } = &*inst {
                            dict.get("_sections").cloned().unwrap_or(py_dict())
                        } else {
                            return Err(PyError::type_error("read_string(): not a ConfigParser instance"));
                        }
                    };

                    let parsed = parse_ini_string(&data);
                    // Merge parsed sections into existing sections
                    if let PyObject::Dict(ref mut sections_dict) = &mut *sections_ref.borrow_mut() {
                        for (section_name, options) in parsed {
                            let section_key = py_str(&section_name);
                            // Try to get existing section dict
                            let existing = sections_dict.get(&section_key).ok().and_then(|o| o);
                            if let Some(existing_ref) = existing {
                                if let PyObject::Dict(ref mut existing_dict) = &mut *existing_ref.borrow_mut() {
                                    for (key, val) in options {
                                        let _ = existing_dict.set(py_str(&key), py_str(&val));
                                    }
                                }
                            } else {
                                // Create new section dict
                                let option_dict = py_dict();
                                if let PyObject::Dict(ref mut od) = &mut *option_dict.borrow_mut() {
                                    for (key, val) in options {
                                        let _ = od.set(py_str(&key), py_str(&val));
                                    }
                                }
                                let _ = sections_dict.set(py_str(&section_name), option_dict);
                            }
                        }
                    }

                    Ok(py_none())
                },
            }));

            // read(self, filename) — parse INI from a file
            type_dict.insert("read".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
                name: "read".to_string(),
                func: |inner_args| {
                    if inner_args.len() < 2 {
                        return Err(PyError::type_error("read() missing required argument: filename"));
                    }
                    let filename = inner_args[1].str();
                    let content = match std::fs::read_to_string(&filename) {
                        Ok(s) => s,
                        Err(e) => return Err(PyError::type_error(format!("Cannot read file '{}': {}", filename, e))),
                    };

                    // Reuse read_string logic — call it on self
                    let sections_ref = {
                        let inst = inner_args[0].borrow();
                        if let PyObject::Instance { dict, .. } = &*inst {
                            dict.get("_sections").cloned().unwrap_or(py_dict())
                        } else {
                            return Err(PyError::type_error("read(): not a ConfigParser instance"));
                        }
                    };

                    let parsed = parse_ini_string(&content);
                    if let PyObject::Dict(ref mut sections_dict) = &mut *sections_ref.borrow_mut() {
                        for (section_name, options) in parsed {
                            let section_key = py_str(&section_name);
                            let existing = sections_dict.get(&section_key).ok().and_then(|o| o);
                            if let Some(existing_ref) = existing {
                                if let PyObject::Dict(ref mut existing_dict) = &mut *existing_ref.borrow_mut() {
                                    for (key, val) in options {
                                        let _ = existing_dict.set(py_str(&key), py_str(&val));
                                    }
                                }
                            } else {
                                let option_dict = py_dict();
                                if let PyObject::Dict(ref mut od) = &mut *option_dict.borrow_mut() {
                                    for (key, val) in options {
                                        let _ = od.set(py_str(&key), py_str(&val));
                                    }
                                }
                                let _ = sections_dict.set(py_str(&section_name), option_dict);
                            }
                        }
                    }

                    // Return list of successfully read files
                    Ok(py_list(vec![inner_args[1].clone()]))
                },
            }));

            // sections(self) — return list of section names
            type_dict.insert("sections".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
                name: "sections".to_string(),
                func: |inner_args| {
                    if inner_args.is_empty() {
                        return Err(PyError::type_error("sections() missing self argument"));
                    }
                    let inst = inner_args[0].borrow();
                    if let PyObject::Instance { dict, .. } = &*inst {
                        let sections_ref = dict.get("_sections").cloned().unwrap_or(py_dict());
                        let sections_borrow = sections_ref.borrow();
                        if let PyObject::Dict(sections_dict) = &*sections_borrow {
                            let mut names: Vec<PyObjectRef> = Vec::new();
                            for (k, _) in sections_dict.items() {
                                let name = k.str();
                                if name != "DEFAULT" {
                                    names.push(py_str(&name));
                                }
                            }
                            Ok(py_list(names))
                        } else {
                            Ok(py_list(vec![]))
                        }
                    } else {
                        Err(PyError::type_error("sections(): not a ConfigParser instance"))
                    }
                },
            }));

            // options(self, section) — return list of option names in a section
            type_dict.insert("options".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
                name: "options".to_string(),
                func: |inner_args| {
                    if inner_args.len() < 2 {
                        return Err(PyError::type_error("options() missing required argument: section"));
                    }
                    let section_name = inner_args[1].str();
                    let inst = inner_args[0].borrow();
                    if let PyObject::Instance { dict, .. } = &*inst {
                        let sections_ref = dict.get("_sections").cloned().unwrap_or(py_dict());
                        drop(inst);
                        let sections_borrow = sections_ref.borrow();
                        if let PyObject::Dict(sections_dict) = &*sections_borrow {
                            let section_key = py_str(&section_name);
                            if let Ok(Some(section_ref)) = sections_dict.get(&section_key) {
                                let section_borrow = section_ref.borrow();
                                if let PyObject::Dict(option_dict) = &*section_borrow {
                                    let mut keys: Vec<PyObjectRef> = option_dict.keys().into_iter()
                                        .map(|k| py_str(&k.str()))
                                        .collect();
                                    // Also include DEFAULT options
                                    if section_name != "DEFAULT" {
                                        if let Ok(Some(default_ref)) = sections_dict.get(&py_str("DEFAULT")) {
                                            if let PyObject::Dict(default_dict) = &*default_ref.borrow() {
                                                for k in default_dict.keys() {
                                                    let kstr = k.str();
                                                    if !keys.iter().any(|k2| k2.str() == kstr) {
                                                        keys.push(py_str(&kstr));
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    Ok(py_list(keys))
                                } else {
                                    Ok(py_list(vec![]))
                                }
                            } else {
                                Err(PyError::type_error(format!("No section '{}'", section_name)))
                            }
                        } else {
                            Ok(py_list(vec![]))
                        }
                    } else {
                        Err(PyError::type_error("options(): not a ConfigParser instance"))
                    }
                },
            }));

            // get(self, section, option, fallback=None) — get a value
            type_dict.insert("get".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
                name: "get".to_string(),
                func: |inner_args| {
                    if inner_args.len() < 3 {
                        return Err(PyError::type_error("get() missing required arguments: section, option"));
                    }
                    let section_name = inner_args[1].str();
                    let option_name = inner_args[2].str();
                    let fallback = if inner_args.len() > 3 { Some(inner_args[3].clone()) } else { None };

                    let inst = inner_args[0].borrow();
                    if let PyObject::Instance { dict, .. } = &*inst {
                        let sections_ref = dict.get("_sections").cloned().unwrap_or(py_dict());
                        drop(inst);

                        let sections_borrowed = sections_ref.borrow();
                        if let PyObject::Dict(sections_dict) = &*sections_borrowed {
                            // Try the specified section
                            let section_key = py_str(&section_name);
                            if let Ok(Some(section_ref)) = sections_dict.get(&section_key) {
                                if let PyObject::Dict(option_dict) = &*section_ref.borrow() {
                                    let option_key = py_str(&option_name);
                                    if let Ok(Some(val)) = option_dict.get(&option_key) {
                                        return Ok(val);
                                    }
                                }
                            }
                            // Try DEFAULT section
                            if section_name != "DEFAULT" {
                                if let Ok(Some(default_ref)) = sections_dict.get(&py_str("DEFAULT")) {
                                    if let PyObject::Dict(default_dict) = &*default_ref.borrow() {
                                        let option_key = py_str(&option_name);
                                        if let Ok(Some(val)) = default_dict.get(&option_key) {
                                            return Ok(val);
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // Return fallback or raise error
                    match fallback {
                        Some(fb) => Ok(fb),
                        None => Err(PyError::type_error(format!(
                            "No option '{}' in section '{}'", option_name, section_name
                        ))),
                    }
                },
            }));

            // items(self, section) — return list of (option, value) tuples
            type_dict.insert("items".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
                name: "items".to_string(),
                func: |inner_args| {
                    if inner_args.len() < 2 {
                        return Err(PyError::type_error("items() missing required argument: section"));
                    }
                    let section_name = inner_args[1].str();
                    let inst = inner_args[0].borrow();
                    if let PyObject::Instance { dict, .. } = &*inst {
                        let sections_ref = dict.get("_sections").cloned().unwrap_or(py_dict());
                        drop(inst);
                        let sections_borrow = sections_ref.borrow();
                        if let PyObject::Dict(sections_dict) = &*sections_borrow {
                            let section_key = py_str(&section_name);
                            if let Ok(Some(section_ref)) = sections_dict.get(&section_key) {
                                let section_borrow = section_ref.borrow();
                                if let PyObject::Dict(option_dict) = &*section_borrow {
                                    let mut result: Vec<PyObjectRef> = Vec::new();
                                    // Include DEFAULT options first
                                    if section_name != "DEFAULT" {
                                        if let Ok(Some(default_ref)) = sections_dict.get(&py_str("DEFAULT")) {
                                            if let PyObject::Dict(default_dict) = &*default_ref.borrow() {
                                                for (k, v) in default_dict.items() {
                                                    result.push(py_tuple(vec![k, v]));
                                                }
                                            }
                                        }
                                    }
                                    // Add section-specific options
                                    for (k, v) in option_dict.items() {
                                        let kstr = k.str();
                                        // Override DEFAULT if present
                                        if let Some(pos) = result.iter().position(|t| {
                                            if let PyObject::Tuple(items) = &*t.borrow() {
                                                items[0].str() == kstr
                                            } else { false }
                                        }) {
                                            result[pos] = py_tuple(vec![k, v]);
                                        } else {
                                            result.push(py_tuple(vec![k, v]));
                                        }
                                    }
                                    Ok(py_list(result))
                                } else {
                                    Ok(py_list(vec![]))
                                }
                            } else {
                                Err(PyError::type_error(format!("No section '{}'", section_name)))
                            }
                        } else {
                            Ok(py_list(vec![]))
                        }
                    } else {
                        Err(PyError::type_error("items(): not a ConfigParser instance"))
                    }
                },
            }));

            // add_section(self, name) — add a new section
            type_dict.insert("add_section".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
                name: "add_section".to_string(),
                func: |inner_args| {
                    if inner_args.len() < 2 {
                        return Err(PyError::type_error("add_section() missing required argument: name"));
                    }
                    let section_name = inner_args[1].str();

                    let sections_ref = {
                        let inst = inner_args[0].borrow();
                        if let PyObject::Instance { dict, .. } = &*inst {
                            dict.get("_sections").cloned().unwrap_or(py_dict())
                        } else {
                            return Err(PyError::type_error("add_section(): not a ConfigParser instance"));
                        }
                    };

                    if let PyObject::Dict(ref mut sections_dict) = &mut *sections_ref.borrow_mut() {
                        let section_key = py_str(&section_name);
                        if sections_dict.contains(&section_key).unwrap_or(false) {
                            return Err(PyError::type_error(format!("Section '{}' already exists", section_name)));
                        }
                        let _ = sections_dict.set(py_str(&section_name), py_dict());
                    }

                    Ok(py_none())
                },
            }));

            // set(self, section, option, value) — set an option
            type_dict.insert("set".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
                name: "set".to_string(),
                func: |inner_args| {
                    if inner_args.len() < 4 {
                        return Err(PyError::type_error("set() missing required arguments: section, option, value"));
                    }
                    let section_name = inner_args[1].str();
                    let option_name = inner_args[2].str();
                    let value = inner_args[3].str();

                    let sections_ref = {
                        let inst = inner_args[0].borrow();
                        if let PyObject::Instance { dict, .. } = &*inst {
                            dict.get("_sections").cloned().unwrap_or(py_dict())
                        } else {
                            return Err(PyError::type_error("set(): not a ConfigParser instance"));
                        }
                    };

                    if let PyObject::Dict(ref mut sections_dict) = &mut *sections_ref.borrow_mut() {
                        let section_key = py_str(&section_name);
                        // Check section exists
                        if !sections_dict.contains(&section_key).unwrap_or(false) {
                            return Err(PyError::type_error(format!("No section '{}'", section_name)));
                        }
                        if let Ok(Some(existing_ref)) = sections_dict.get(&section_key) {
                            if let PyObject::Dict(ref mut option_dict) = &mut *existing_ref.borrow_mut() {
                                let _ = option_dict.set(py_str(&option_name), py_str(&value));
                            }
                        }
                    }

                    Ok(py_none())
                },
            }));

            // has_section(self, name) — check if section exists
            type_dict.insert("has_section".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
                name: "has_section".to_string(),
                func: |inner_args| {
                    if inner_args.len() < 2 {
                        return Err(PyError::type_error("has_section() missing required argument: name"));
                    }
                    let section_name = inner_args[1].str();
                    let inst = inner_args[0].borrow();
                    if let PyObject::Instance { dict, .. } = &*inst {
                        let sections_ref = dict.get("_sections").cloned().unwrap_or(py_dict());
                        drop(inst);
                        let sections_borrow = sections_ref.borrow();
                        if let PyObject::Dict(sections_dict) = &*sections_borrow {
                            let section_key = py_str(&section_name);
                            let found = sections_dict.contains(&section_key).unwrap_or(false);
                            Ok(py_bool(found))
                        } else {
                            Ok(py_bool(false))
                        }
                    } else {
                        Err(PyError::type_error("has_section(): not a ConfigParser instance"))
                    }
                },
            }));

            let typ = PyObjectRef::new(PyObject::Type {
                name: "ConfigParser".to_string(),
                dict: type_dict,
                bases: vec![],
                mro: vec![],
            });

            let mut instance_dict = HashMap::new();
            instance_dict.insert("_sections".to_string(), py_dict());

            Ok(PyObjectRef::new(PyObject::Instance {
                typ,
                dict: instance_dict,
            }))
        },
    }));

    d
}

use std::rc::Rc;
use std::cell::RefCell;
use std::sync::{Arc, Mutex};
use num_traits::ToPrimitive;
use num_bigint::BigInt;
use std::sync::atomic::{AtomicI64, Ordering};
use crate::bytecode::{needs_arg, CodeObject};

// ---------------------------------------------------------------------------
// numbers module — Number ABCs as py_str stubs
// ---------------------------------------------------------------------------
pub fn create_numbers_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    // Number ABCs — simple string stubs (matchable via isinstance checks later)
    d.insert("Number".to_string(), py_str("Number"));
    d.insert("Complex".to_string(), py_str("Complex"));
    d.insert("Real".to_string(), py_str("Real"));
    d.insert("Rational".to_string(), py_str("Rational"));
    d.insert("Integral".to_string(), py_str("Integral"));
    d
}

// ---------------------------------------------------------------------------
// ast module — literal_eval and basic AST node stubs
// ---------------------------------------------------------------------------
pub fn create_ast_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! ast_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    // literal_eval — simplified parser handling common Python literals
    ast_func!("literal_eval", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("literal_eval() missing required argument: node_or_string"));
        }
        let arg = &args[0];
        let s = arg.str();
        // Trim whitespace
        let s = s.trim().to_string();
        if s.is_empty() {
            return Err(PyError::ValueError("malformed node or string: empty literal".to_string()));
        }

        // Try parsing as a literal from left to right
        let chars: Vec<char> = s.chars().collect();
        let mut pos = 0;
        let result = parse_literal(&chars, &mut pos)?;
        // Expect EOF after successful parse
        skip_ws(&chars, &mut pos);
        if pos < chars.len() {
            return Err(PyError::ValueError(format!("malformed node or string: trailing garbage at position {}", pos)));
        }
        Ok(result)
    });

    d.insert("AST".to_string(), py_str("AST"));
    d.insert("Node".to_string(), py_str("Node"));
    d.insert("Expr".to_string(), py_str("Expr"));
    d.insert("Module".to_string(), py_str("Module"));
    d.insert("Load".to_string(), py_str("Load"));
    d.insert("Store".to_string(), py_str("Store"));
    d.insert("Del".to_string(), py_str("Del"));
    d.insert("Pass".to_string(), py_str("Pass"));
    d.insert("Break".to_string(), py_str("Break"));
    d.insert("Continue".to_string(), py_str("Continue"));

    d
}

/// Skip whitespace characters in the character slice.
fn skip_ws(chars: &[char], pos: &mut usize) {
    while *pos < chars.len() && chars[*pos].is_ascii_whitespace() {
        *pos += 1;
    }
}

/// Parse a single Python literal starting at `pos`.  Supports: strings,
/// integers, floats, True, False, None, tuples (...), lists [...], dicts {...}.
fn parse_literal(chars: &[char], pos: &mut usize) -> PyResult<PyObjectRef> {
    skip_ws(chars, pos);
    if *pos >= chars.len() {
        return Err(PyError::ValueError("malformed node or string: unexpected end".to_string()));
    }

    match chars[*pos] {
        // String literal: simple quoted string (no escape sequences)
        '\'' | '"' => {
            let quote = chars[*pos];
            *pos += 1;
            let mut buf = String::new();
            loop {
                if *pos >= chars.len() {
                    return Err(PyError::ValueError("malformed node or string: unterminated string".to_string()));
                }
                let c = chars[*pos];
                *pos += 1;
                if c == quote {
                    break;
                }
                if c == '\\' && *pos < chars.len() {
                    // Handle common escape sequences
                    let next = chars[*pos];
                    *pos += 1;
                    match next {
                        'n' => buf.push('\n'),
                        't' => buf.push('\t'),
                        'r' => buf.push('\r'),
                        '\\' => buf.push('\\'),
                        '\'' => buf.push('\''),
                        '"' => buf.push('"'),
                        _ => {
                            buf.push('\\');
                            buf.push(next);
                        }
                    }
                } else {
                    buf.push(c);
                }
            }
            Ok(py_str(&buf))
        }
        // Tuple
        '(' => {
            *pos += 1;
            let mut items = Vec::new();
            skip_ws(chars, pos);
            if *pos < chars.len() && chars[*pos] == ')' {
                *pos += 1;
                return Ok(py_tuple(items));
            }
            loop {
                skip_ws(chars, pos);
                let item = parse_literal(chars, pos)?;
                items.push(item);
                skip_ws(chars, pos);
                if *pos >= chars.len() {
                    return Err(PyError::ValueError("malformed node or string: unterminated tuple".to_string()));
                }
                if chars[*pos] == ')' {
                    *pos += 1;
                    break;
                }
                if chars[*pos] != ',' {
                    return Err(PyError::ValueError("malformed node or string: expected ',' or ')' in tuple".to_string()));
                }
                *pos += 1;
            }
            Ok(py_tuple(items))
        }
        // List
        '[' => {
            *pos += 1;
            let mut items = Vec::new();
            skip_ws(chars, pos);
            if *pos < chars.len() && chars[*pos] == ']' {
                *pos += 1;
                return Ok(py_list(items));
            }
            loop {
                skip_ws(chars, pos);
                let item = parse_literal(chars, pos)?;
                items.push(item);
                skip_ws(chars, pos);
                if *pos >= chars.len() {
                    return Err(PyError::ValueError("malformed node or string: unterminated list".to_string()));
                }
                if chars[*pos] == ']' {
                    *pos += 1;
                    break;
                }
                if chars[*pos] != ',' {
                    return Err(PyError::ValueError("malformed node or string: expected ',' or ']' in list".to_string()));
                }
                *pos += 1;
            }
            Ok(py_list(items))
        }
        // Dict
        '{' => {
            *pos += 1;
            let dict_obj = py_dict();
            skip_ws(chars, pos);
            if *pos < chars.len() && chars[*pos] == '}' {
                *pos += 1;
                return Ok(dict_obj);
            }
            loop {
                skip_ws(chars, pos);
                let key = parse_literal(chars, pos)?;
                skip_ws(chars, pos);
                if *pos >= chars.len() || chars[*pos] != ':' {
                    return Err(PyError::ValueError("malformed node or string: expected ':' in dict literal".to_string()));
                }
                *pos += 1;
                skip_ws(chars, pos);
                let value = parse_literal(chars, pos)?;
                // Set key-value in dict object
                let key_str = key.str();
                if let PyObject::Dict(ref mut d) = *dict_obj.borrow_mut() {
                    d.set(py_str(&key_str), value).ok();
                }
                skip_ws(chars, pos);
                if *pos >= chars.len() {
                    return Err(PyError::ValueError("malformed node or string: unterminated dict".to_string()));
                }
                if chars[*pos] == '}' {
                    *pos += 1;
                    break;
                }
                if chars[*pos] != ',' {
                    return Err(PyError::ValueError("malformed node or string: expected ',' or '}' in dict".to_string()));
                }
                *pos += 1;
            }
            Ok(dict_obj)
        }
        // Number or keyword literal
        _ => {
            let start = *pos;
            let mut buf = String::new();
            // Accumulate identifier-like or number characters
            while *pos < chars.len() {
                let c = chars[*pos];
                if c.is_alphanumeric() || c == '.' || c == '_' || c == '-' || c == '+' {
                    // For negative/positive numbers, handle the sign
                    if (c == '-' || c == '+') && !buf.is_empty() && buf != "-" && buf != "+" {
                        // Signs allowed only at the start or after 'e'/'E'
                        if buf.ends_with('e') || buf.ends_with('E') {
                            buf.push(c);
                            *pos += 1;
                        } else {
                            break;
                        }
                    } else {
                        buf.push(c);
                        *pos += 1;
                    }
                } else {
                    break;
                }
            }
            if buf.is_empty() {
                return Err(PyError::ValueError(format!(
                    "malformed node or string: unexpected character '{}' at position {}",
                    chars[*pos], *pos
                )));
            }
            // Check keywords
            match buf.as_str() {
                "True" => return Ok(py_bool(true)),
                "False" => return Ok(py_bool(false)),
                "None" => return Ok(py_none()),
                _ => {}
            }
            // Check for float (contains '.')
            if buf.contains('.') || buf.contains('e') || buf.contains('E') {
                match buf.parse::<f64>() {
                    Ok(v) => Ok(py_float(v)),
                    Err(_) => Err(PyError::ValueError(format!("malformed node or string: invalid float literal '{}'", buf))),
                }
            } else {
                // Integer
                let clean = buf.replace('_', "");
                if clean.starts_with("0x") || clean.starts_with("0X") {
                    match i64::from_str_radix(&clean[2..], 16) {
                        Ok(v) => Ok(py_int(v)),
                        Err(_) => Err(PyError::ValueError(format!("malformed node or string: invalid hex literal '{}'", buf))),
                    }
                } else if clean.starts_with("0o") || clean.starts_with("0O") {
                    match i64::from_str_radix(&clean[2..], 8) {
                        Ok(v) => Ok(py_int(v)),
                        Err(_) => Err(PyError::ValueError(format!("malformed node or string: invalid octal literal '{}'", buf))),
                    }
                } else if clean.starts_with("0b") || clean.starts_with("0B") {
                    match i64::from_str_radix(&clean[2..], 2) {
                        Ok(v) => Ok(py_int(v)),
                        Err(_) => Err(PyError::ValueError(format!("malformed node or string: invalid binary literal '{}'", buf))),
                    }
                } else {
                    match clean.parse::<i64>() {
                        Ok(v) => Ok(py_int(v)),
                        Err(_) => Err(PyError::ValueError(format!("malformed node or string: invalid integer literal '{}'", buf))),
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// sunau module — AU audio file format stub
// ---------------------------------------------------------------------------
pub fn create_sunau_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! sunau_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    // Error types
    d.insert("Error".to_string(), py_str("Error"));
    d.insert("Au_read".to_string(), py_str("Au_read"));

    // Constants (Sun AU file format)
    d.insert("MAGIC".to_string(), py_int(0x2e736e64)); // ".snd" magic
    d.insert("SND_MAGIC".to_string(), py_int(0x2e736e64));
    d.insert("SND_HEADER_SIZE".to_string(), py_int(24));

    // Encoding constants
    d.insert("ULAW".to_string(), py_int(1));
    d.insert("LINEAR8".to_string(), py_int(2));
    d.insert("LINEAR16".to_string(), py_int(3));
    d.insert("LINEAR24".to_string(), py_int(4));
    d.insert("LINEAR32".to_string(), py_int(5));
    d.insert("FLOAT".to_string(), py_int(6));
    d.insert("DOUBLE".to_string(), py_int(7));
    d.insert("ADPCM_G721".to_string(), py_int(23));
    d.insert("ADPCM_G722".to_string(), py_int(24));
    d.insert("ADPCM_G723_3".to_string(), py_int(25));
    d.insert("ADPCM_G723_5".to_string(), py_int(26));
    d.insert("ALAW_8".to_string(), py_int(27));

    // open() — returns an Au_read stub
    sunau_func!("open", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("open() missing required argument: file"));
        }
        // Return a minimal Au_read object stub
        let mut instance_dict = HashMap::new();
        instance_dict.insert("nchannels".to_string(), py_int(1));
        instance_dict.insert("sampwidth".to_string(), py_int(2));
        instance_dict.insert("framerate".to_string(), py_int(8000));
        instance_dict.insert("nframes".to_string(), py_int(0));
        instance_dict.insert("encoding".to_string(), py_int(1)); // ULAW
        instance_dict.insert("_file".to_string(), args[0].clone());

        let mut type_dict: HashMap<String, PyObjectRef> = HashMap::new();
        type_dict.insert("getnchannels".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
            name: "getnchannels".to_string(),
            func: |self_args| {
                if self_args.is_empty() {
                    return Err(PyError::type_error("getnchannels() missing self"));
                }
                let inst = self_args[0].borrow();
                if let PyObject::Instance { dict, .. } = &*inst {
                    Ok(dict.get("nchannels").cloned().unwrap_or(py_int(1)))
                } else {
                    Err(PyError::type_error("getnchannels: not an Au_read instance"))
                }
            },
        }));
        type_dict.insert("getsampwidth".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
            name: "getsampwidth".to_string(),
            func: |self_args| {
                if self_args.is_empty() {
                    return Err(PyError::type_error("getsampwidth() missing self"));
                }
                let inst = self_args[0].borrow();
                if let PyObject::Instance { dict, .. } = &*inst {
                    Ok(dict.get("sampwidth").cloned().unwrap_or(py_int(2)))
                } else {
                    Err(PyError::type_error("getsampwidth: not an Au_read instance"))
                }
            },
        }));
        type_dict.insert("getframerate".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
            name: "getframerate".to_string(),
            func: |self_args| {
                if self_args.is_empty() {
                    return Err(PyError::type_error("getframerate() missing self"));
                }
                let inst = self_args[0].borrow();
                if let PyObject::Instance { dict, .. } = &*inst {
                    Ok(dict.get("framerate").cloned().unwrap_or(py_int(8000)))
                } else {
                    Err(PyError::type_error("getframerate: not an Au_read instance"))
                }
            },
        }));
        type_dict.insert("getnframes".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
            name: "getnframes".to_string(),
            func: |self_args| {
                if self_args.is_empty() {
                    return Err(PyError::type_error("getnframes() missing self"));
                }
                let inst = self_args[0].borrow();
                if let PyObject::Instance { dict, .. } = &*inst {
                    Ok(dict.get("nframes").cloned().unwrap_or(py_int(0)))
                } else {
                    Err(PyError::type_error("getnframes: not an Au_read instance"))
                }
            },
        }));
        type_dict.insert("getcomptype".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
            name: "getcomptype".to_string(),
            func: |_| Ok(py_str("NONE")),
        }));
        type_dict.insert("getcompname".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
            name: "getcompname".to_string(),
            func: |_| Ok(py_str("not compressed")),
        }));
        type_dict.insert("close".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
            name: "close".to_string(),
            func: |_| Ok(py_none()),
        }));

        let typ = PyObjectRef::new(PyObject::Type {
            name: "Au_read".to_string(),
            dict: type_dict,
            bases: vec![],
            mro: vec![],
        });

        Ok(PyObjectRef::new(PyObject::Instance { typ, dict: instance_dict }))
    });

    d
}

// ─── xml.etree.ElementTree module ─────────────────────────────────────────────

thread_local! {
    static ELEMENT_TYPE: std::cell::RefCell<Option<PyObjectRef>> = const { std::cell::RefCell::new(None) };
}

pub fn create_xml_etree_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! et_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    // Build Element type with methods
    let mut element_type_dict = HashMap::new();
    macro_rules! e_method {
        ($name:expr, $func:expr) => {
            element_type_dict.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    e_method!("append", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("append() takes 1 argument (Element)"));
        }
        let child = args[1].clone();
        let list = {
            let obj = args[0].borrow();
            if let PyObject::Instance { dict, .. } = &*obj {
                dict.get("children").cloned()
            } else {
                None
            }
        };
        if let Some(children) = list {
            if let PyObject::List(lst) = &mut *children.borrow_mut() {
                lst.push(child);
                return Ok(py_none());
            }
        }
        Err(PyError::type_error("append: self is not an Element"))
    });

    e_method!("find", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("find() takes 1 argument"));
        }
        let path = args[1].str();
        let obj = args[0].borrow();
        if let PyObject::Instance { dict, .. } = &*obj {
            if let Some(children) = dict.get("children") {
                if let PyObject::List(list) = &*children.borrow() {
                    for child in list {
                        let c = child.borrow();
                        if let PyObject::Instance { dict: cd, .. } = &*c {
                            if let Some(tag) = cd.get("tag") {
                                if tag.str() == path {
                                    return Ok(child.clone());
                                }
                            }
                        }
                    }
                }
            }
        }
        Ok(py_none())
    });

    e_method!("findall", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("findall() takes 1 argument"));
        }
        let path = args[1].str();
        let results = py_list(vec![]);
        let obj = args[0].borrow();
        if let PyObject::Instance { dict, .. } = &*obj {
            if let Some(children) = dict.get("children") {
                if let PyObject::List(list) = &*children.borrow() {
                    for child in list {
                        let c = child.borrow();
                        if let PyObject::Instance { dict: cd, .. } = &*c {
                            if let Some(tag) = cd.get("tag") {
                                if tag.str() == path {
                                    if let PyObject::List(rl) = &mut *results.borrow_mut() {
                                        rl.push(child.clone());
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        Ok(results)
    });

    e_method!("get", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("get() takes at least 1 argument"));
        }
        let key = args[1].str();
        let default = if args.len() > 2 { Some(args[2].clone()) } else { None };
        let obj = args[0].borrow();
        if let PyObject::Instance { dict, .. } = &*obj {
            if let Some(attrib) = dict.get("attrib") {
                if let PyObject::Dict(ad) = &*attrib.borrow() {
                    for (k, v) in ad.items() {
                        if k.str() == key {
                            return Ok(v);
                        }
                    }
                }
            }
        }
        Ok(default.unwrap_or(py_none()))
    });

    e_method!("items", |args| {
        let obj = args[0].borrow();
        if let PyObject::Instance { dict, .. } = &*obj {
            if let Some(attrib) = dict.get("attrib") {
                if let PyObject::Dict(ad) = &*attrib.borrow() {
                    let mut items = vec![];
                    for (k, v) in ad.items() {
                        items.push(py_tuple(vec![k, v]));
                    }
                    return Ok(py_list(items));
                }
            }
        }
        Ok(py_list(vec![]))
    });

    e_method!("keys", |args| {
        let obj = args[0].borrow();
        if let PyObject::Instance { dict, .. } = &*obj {
            if let Some(attrib) = dict.get("attrib") {
                if let PyObject::Dict(ad) = &*attrib.borrow() {
                    return Ok(py_list(ad.keys()));
                }
            }
        }
        Ok(py_list(vec![]))
    });

    let element_type = PyObjectRef::new(PyObject::Type {
        name: "Element".to_string(),
        dict: element_type_dict,
        bases: vec![],
        mro: vec![],
    });

    // Store element type in thread-local for factory functions to use
    ELEMENT_TYPE.with(|cache| {
        *cache.borrow_mut() = Some(element_type.clone());
    });

    // Helper to create a new Element instance
    fn new_element(tag: &str) -> PyObjectRef {
        let typ = ELEMENT_TYPE.with(|cache| {
            cache.borrow().clone().unwrap()
        });
        let mut instance_dict = HashMap::new();
        instance_dict.insert("tag".to_string(), py_str(tag));
        instance_dict.insert("text".to_string(), py_none());
        instance_dict.insert("attrib".to_string(), py_dict());
        instance_dict.insert("children".to_string(), py_list(vec![]));
        PyObjectRef::new(PyObject::Instance { typ, dict: instance_dict })
    }

    // Element(tag) factory
    et_func!("Element", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("Element() missing tag argument"));
        }
        let tag = args[0].str();
        Ok(new_element(&tag))
    });

    // SubElement(parent, tag) factory
    et_func!("SubElement", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("SubElement() requires at least 2 arguments"));
        }
        let parent = &args[0];
        let tag = args[1].str();
        let child = new_element(&tag);
        // Append to parent's children list
        let list = {
            let obj = parent.borrow();
            if let PyObject::Instance { dict, .. } = &*obj {
                dict.get("children").cloned()
            } else {
                None
            }
        };
        if let Some(children) = list {
            if let PyObject::List(lst) = &mut *children.borrow_mut() {
                lst.push(child.clone());
            }
        }
        Ok(child)
    });

    // tostring(el) — serialize to XML string
    fn serialize_element(obj: &PyObjectRef) -> String {
        let (tag, text, children) = {
            let b = obj.borrow();
            if let PyObject::Instance { dict, .. } = &*b {
                let t = dict.get("tag").map(|t| t.str()).unwrap_or_default();
                let txt = dict.get("text").and_then(|t| {
                    let s = t.str();
                    if s.is_empty() || s == "None" { None } else { Some(s) }
                });
                let kids = dict.get("children").and_then(|c| {
                    if let PyObject::List(list) = &*c.borrow() {
                        Some(list.clone())
                    } else {
                        None
                    }
                }).unwrap_or_default();
                (t, txt, kids)
            } else {
                (String::new(), None, vec![])
            }
        };
        if children.is_empty() && text.is_none() {
            format!("<{} />", tag)
        } else {
            let mut result = format!("<{}>", tag);
            if let Some(t) = text {
                result.push_str(&t.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;"));
            }
            for child in &children {
                result.push_str(&serialize_element(child));
            }
            result.push_str(&format!("</{}>", tag));
            result
        }
    }

    et_func!("tostring", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("tostring() missing required argument"));
        }
        Ok(py_str(&serialize_element(&args[0])))
    });

    // fromstring(xml_str) — parse simple XML
    fn parse_xml(s: &str, pos: &mut usize) -> Option<PyObjectRef> {
        // Skip whitespace
        while *pos < s.len() && s.as_bytes()[*pos].is_ascii_whitespace() {
            *pos += 1;
        }
        if *pos >= s.len() || s.as_bytes()[*pos] != b'<' {
            return None;
        }
        *pos += 1; // skip '<'
        // Check for closing tag
        if *pos < s.len() && s.as_bytes()[*pos] == b'/' {
            return None;
        }
        // Read tag name
        let start = *pos;
        while *pos < s.len() && !s.as_bytes()[*pos].is_ascii_whitespace() && s.as_bytes()[*pos] != b'>' && s.as_bytes()[*pos] != b'/' {
            *pos += 1;
        }
        let tag_name = &s[start..*pos];
        // Skip attributes (not parsed in depth)
        while *pos < s.len() && s.as_bytes()[*pos] != b'>' && s.as_bytes()[*pos] != b'/' {
            *pos += 1;
        }
        // Self-closing tag
        if *pos < s.len() && s.as_bytes()[*pos] == b'/' {
            *pos += 2; // skip '/>'
            return Some(new_element(tag_name));
        }
        // Skip '>'
        if *pos < s.len() && s.as_bytes()[*pos] == b'>' {
            *pos += 1;
        }
        let el = new_element(tag_name);
        // Read children/text until closing tag
        let mut text = String::new();
        loop {
            while *pos < s.len() && s.as_bytes()[*pos].is_ascii_whitespace() {
                text.push(s.as_bytes()[*pos] as char);
                *pos += 1;
            }
            if *pos >= s.len() {
                break;
            }
            if s.as_bytes()[*pos] == b'<' {
                if *pos + 1 < s.len() && s.as_bytes()[*pos + 1] == b'/' {
                    *pos += 2; // skip '</'
                    while *pos < s.len() && s.as_bytes()[*pos] != b'>' {
                        *pos += 1;
                    }
                    if *pos < s.len() {
                        *pos += 1; // skip '>'
                    }
                    let trimmed = text.trim();
                    if !trimmed.is_empty() {
                        if let PyObject::Instance { dict, .. } = &mut *el.borrow_mut() {
                            dict.insert("text".to_string(), py_str(trimmed));
                        }
                    }
                    return Some(el);
                }
                // Parse child element
                if let Some(child) = parse_xml(s, pos) {
                    let list = {
                        let obj = el.borrow();
                        if let PyObject::Instance { dict, .. } = &*obj {
                            dict.get("children").cloned()
                        } else {
                            None
                        }
                    };
                    if let Some(children) = list {
                        if let PyObject::List(lst) = &mut *children.borrow_mut() {
                            lst.push(child);
                        }
                    }
                } else {
                    break;
                }
            } else {
                text.push(s.as_bytes()[*pos] as char);
                *pos += 1;
            }
        }
        Some(el)
    }

    et_func!("fromstring", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("fromstring() missing required argument"));
        }
        let xml_str = args[0].str();
        let mut pos = 0;
        match parse_xml(&xml_str, &mut pos) {
            Some(el) => Ok(el),
            None => Err(PyError::type_error("fromstring: could not parse XML")),
        }
    });

    d
}

// ─── xml module (empty package) ───────────────────────────────────────────────

pub fn create_xml_dict() -> HashMap<String, PyObjectRef> {
    HashMap::new()
}

// ─── this module (Zen of Python) ──────────────────────────────────────────────

pub fn create_this_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    let zen = "Beautiful is better than ugly.\n\
               Explicit is better than implicit.\n\
               Simple is better than complex.\n\
               Complex is better than complicated.\n\
               Flat is better than nested.\n\
               Sparse is better than dense.\n\
               Readability counts.\n\
               Special cases aren't special enough to break the rules.\n\
               Although practicality beats purity.\n\
               Errors should never pass silently.\n\
               Unless explicitly silenced.\n\
               In the face of ambiguity, refuse the temptation to guess.\n\
               There should be one-- and preferably only one --obvious way to do it.\n\
               Although that way may not be obvious at first unless you're Dutch.\n\
               Now is better than never.\n\
               Although never is often better than *right* now.\n\
               If the implementation is hard to explain, it's a bad idea.\n\
               If the implementation is easy to explain, it may be a good idea.\n\
               Namespaces are one honking great idea -- let's do more of those!";
    // Store Zen text as module data (prints on explicit import, not at startup)
    d.insert("s".to_string(), py_str(zen));
    d
}

// ─── argparse module ──────────────────────────────────────────────────────────

pub fn create_argparse_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    let mut parser_type_dict = HashMap::new();
    macro_rules! p_method {
        ($name:expr, $func:expr) => {
            parser_type_dict.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    p_method!("__init__", |args| {
        // Accept optional description (first arg after self)
        // self is args[0], description would be args[1]
        Ok(py_none())
    });

    p_method!("add_argument", |args| {
        // Stub: return None
        Ok(py_none())
    });

    p_method!("parse_args", |args| {
        // Create Namespace instance
        let ns_type = PyObjectRef::new(PyObject::Type {
            name: "Namespace".to_string(),
            dict: HashMap::new(),
            bases: vec![],
            mro: vec![],
        });

        let mut ns_dict = HashMap::new();
        if args.len() > 1 {
            let arg_list: Vec<String> = {
                let borrowed = args[1].borrow();
                if let PyObject::List(list) = &*borrowed {
                    list.iter().map(|s| s.str()).collect()
                } else {
                    return Err(PyError::type_error("parse_args: expected a list of strings"));
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

        Ok(PyObjectRef::new(PyObject::Instance { typ: ns_type, dict: ns_dict }))
    });

    let parser_type = PyObjectRef::new(PyObject::Type {
        name: "ArgumentParser".to_string(),
        dict: parser_type_dict,
        bases: vec![],
        mro: vec![],
    });

    d.insert("ArgumentParser".to_string(), parser_type);
    d
}


// ─── asyncio module (basic event loop) ────────────────────────────────────

pub fn create_asyncio_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! asyncio_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    // Future class
    let mut future_type_dict = HashMap::new();
    macro_rules! future_method {
        ($name:expr, $func:expr) => {
            future_type_dict.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }
    future_method!("__init__", |args| {
        let self_obj = args[0].clone();
        let obj = self_obj.borrow_mut();
        // Future state stored in __dict__
        Ok(crate::object::py_none())
    });
    future_method!("__await__", |args| {
        // Returns a generator that yields self once then returns result
        let self_obj = args[0].clone();
        Ok(self_obj)
    });
    future_method!("set_result", |args| {
        let self_obj = args[0].clone();
        let result = args[1].clone();
        self_obj.borrow_mut().set_attribute("_result", result).ok();
        self_obj.borrow_mut().set_attribute("_done", crate::object::py_bool(true)).ok();
        Ok(crate::object::py_none())
    });
    future_method!("done", |args| {
        let self_obj = args[0].clone();
        if let Ok(val) = self_obj.borrow().get_attribute("_done") {
            return Ok(val);
        }
        Ok(crate::object::py_bool(false))
    });
    future_method!("result", |args| {
        let self_obj = args[0].clone();
        if let Ok(val) = self_obj.borrow().get_attribute("_result") {
            return Ok(val);
        }
        Err(crate::object::PyError::runtime_error("Future has no result"))
    });

    let future_type = PyObjectRef::new(PyObject::Type {
        name: "Future".to_string(),
        dict: future_type_dict,
        bases: vec![],
        mro: vec![],
    });
    d.insert("Future".to_string(), future_type);

    // Task class
    let mut task_type_dict = HashMap::new();
    macro_rules! task_method {
        ($name:expr, $func:expr) => {
            task_type_dict.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }
    task_method!("__init__", |args| {
        let self_obj = args[0].clone();
        let coro = args[1].clone();
        self_obj.borrow_mut().set_attribute("_coro", coro).ok();
        self_obj.borrow_mut().set_attribute("_done", crate::object::py_bool(false)).ok();
        Ok(crate::object::py_none())
    });
    task_method!("step", |args| {
        let self_obj = args[0].clone();
        let coro = self_obj.borrow().get_attribute("_coro")?;
        // Try to advance the coroutine via __next__ or send
        let next_func = coro.borrow().get_attribute("__next__")?;
        match crate::object::call_bound_method(next_func, coro.clone(), vec![]) {
            Ok(val) => {
                // If the coroutine yielded a Future, set up wakeup
                let type_name = val.borrow().type_name();
                if type_name == "Future" {
                    // Register a callback to resume this task
                    let self_clone = self_obj.clone();
                    let callback = PyObjectRef::new(PyObject::Closure(Rc::new(move |_args| {
                        // Step the task again
                        let next_func2 = self_clone.borrow().get_attribute("_coro").ok()
                            .and_then(|c| c.borrow().get_attribute("send").ok());
                        Ok(crate::object::py_none())
                    })));
                    val.borrow_mut().set_attribute("_callbacks", crate::object::py_list(vec![callback])).ok();
                }
                Ok(val)
            }
            Err(crate::object::PyError::StopIteration) => {
                self_obj.borrow_mut().set_attribute("_done", crate::object::py_bool(true)).ok();
                Ok(crate::object::py_none())
            }
            Err(e) => Err(e),
        }
    });

    let task_type = PyObjectRef::new(PyObject::Type {
        name: "Task".to_string(),
        dict: task_type_dict,
        bases: vec![],
        mro: vec![],
    });
    d.insert("Task".to_string(), task_type);

    // asyncio.run(coro): Minimal event loop
    asyncio_func!("run", |args| {
        let coro = args[0].clone();
        // Get the global VM pointer
        let vm_ptr = crate::object::VM_PTR.with(|p| *p.borrow()).ok_or_else(|| {
            crate::object::PyError::runtime_error("no active VM")
        })?;
        // Safety: VM is guaranteed alive during this call
        let vm = unsafe { &mut *vm_ptr };
        // Create a new frame to run the coroutine
        let coro_borrowed = coro.borrow();
        if let crate::object::PyObject::Coroutine { ref frame } = &*coro_borrowed {
            let frame_borrowed = frame.borrow();
            if let Some(ref coro_frame) = *frame_borrowed {
                // We need to push this frame onto the VM and execute
                let mut coro_frame_clone = coro_frame.clone();
                coro_frame_clone.module_globals = None;
                // Execute until completion
                vm.frames.push(coro_frame_clone);
                let result = vm.execute();
                vm.frames.pop();
                return result;
            }
        }
        // If not a coroutine, try calling it directly
        let coro_clone = coro.clone();
        let send_attr = coro_clone.borrow().get_attribute("send").ok();
        if let Some(send_method) = send_attr {
            let result = crate::object::call_bound_method(send_method, coro.clone(), vec![crate::object::py_none()]);
            match result {
                Ok(val) => Ok(val),
                Err(crate::object::PyError::StopIteration) => Ok(crate::object::py_none()),
                Err(e) => Err(e),
            }
        } else {
            crate::object::call_bound_method(coro.clone(), coro.clone(), vec![])
        }
    });

    // asyncio.sleep(delay) -> Future
    // Returns a Future that resolves after the delay
    asyncio_func!("sleep", |args| {
        let delay = args[0].clone();
        // Create a Future by calling builtins.dict or using construct
        let future = crate::object::PyObjectRef::new(crate::object::PyObject::Instance {
            typ: crate::object::py_none(),  // placeholder
            dict: std::collections::HashMap::new(),
        });
        // Set Future-specific attributes
        future.borrow_mut().set_attribute("_done", crate::object::py_bool(false)).ok();
        future.borrow_mut().set_attribute("_result", crate::object::py_none()).ok();
        // For now, immediately resolve sleep(0) and create pending for others
        if let crate::object::PyObject::Int(n) = &*delay.borrow() {
            if n == &num_bigint::BigInt::from(0) {
                future.borrow_mut().set_attribute("_done", crate::object::py_bool(true)).ok();
                future.borrow_mut().set_attribute("_result", crate::object::py_none()).ok();
            }
        }
        Ok(future)
    });

    // asyncio.gather(*coros, return_exceptions=False)
    asyncio_func!("gather", |args| {
        let futures: Vec<PyObjectRef> = args.to_vec();
        // For now, return a simple list of results (blocking gather)
        let mut results = Vec::new();
        for f in &futures {
            // Try to run directly if it's a coroutine
            let f_type = f.borrow().type_name();
            if f_type == "coroutine" || f_type == "generator" {
                if let Ok(send) = f.borrow().get_attribute("send") {
                    match crate::object::call_bound_method(send, f.clone(), vec![crate::object::py_none()]) {
                        Ok(val) => results.push(val),
                        Err(crate::object::PyError::StopIteration) => results.push(crate::object::py_none()),
                        Err(e) => return Err(e),
                    }
                }
            } else {
                results.push(f.clone());
            }
        }
        Ok(crate::object::py_list(results))
    });

    d
}

pub fn create_ssl_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! ssl_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    // Version constants
    d.insert("OPENSSL_VERSION".to_string(), py_str("OpenSSL 3.0.13 30 Jan 2024"));
    d.insert("OPENSSL_VERSION_INFO".to_string(), py_list(vec![py_int(3), py_int(0), py_int(13), py_int(0), py_int(0)]));
    d.insert("OPENSSL_VERSION_NUMBER".to_string(), py_int(0x300000f0));

    // Feature flags
    d.insert("HAS_SNI".to_string(), py_bool(true));
    d.insert("HAS_ALPN".to_string(), py_bool(true));
    d.insert("HAS_TLSv1_3".to_string(), py_bool(true));
    d.insert("HAS_SSLv2".to_string(), py_bool(false));
    d.insert("HAS_SSLv3".to_string(), py_bool(false));
    d.insert("HAS_ECDH".to_string(), py_bool(true));
    d.insert("HAS_NPN".to_string(), py_bool(false));

    // Certificate verification constants
    d.insert("CERT_NONE".to_string(), py_int(0));
    d.insert("CERT_OPTIONAL".to_string(), py_int(1));
    d.insert("CERT_REQUIRED".to_string(), py_int(2));

    // Protocol constants
    d.insert("PROTOCOL_TLS".to_string(), py_int(2));
    d.insert("PROTOCOL_TLS_CLIENT".to_string(), py_int(5));
    d.insert("PROTOCOL_TLS_SERVER".to_string(), py_int(4));
    d.insert("PROTOCOL_SSLv23".to_string(), py_int(2));
    d.insert("PROTOCOL_SSLv3".to_string(), py_int(3));

    // SSL options
    d.insert("OP_ALL".to_string(), py_int(0x80000));
    d.insert("OP_NO_SSLv2".to_string(), py_int(0x100));
    d.insert("OP_NO_SSLv3".to_string(), py_int(0x200));
    d.insert("OP_NO_TLSv1".to_string(), py_int(0x400));
    d.insert("OP_NO_TLSv1_1".to_string(), py_int(0x800));
    d.insert("OP_NO_TLSv1_2".to_string(), py_int(0x1000));
    d.insert("OP_NO_TLSv1_3".to_string(), py_int(0x2000));
    d.insert("OP_SINGLE_DH_USE".to_string(), py_int(0x100000));
    d.insert("OP_SINGLE_ECDH_USE".to_string(), py_int(0x80000));
    d.insert("OP_CIPHER_SERVER_PREFERENCE".to_string(), py_int(0x400000));
    d.insert("OP_NO_COMPRESSION".to_string(), py_int(0x20000));

    // Alert description constants
    d.insert("ALERT_DESCRIPTION_CLOSE_NOTIFY".to_string(), py_int(0));
    d.insert("ALERT_DESCRIPTION_HANDSHAKE_FAILURE".to_string(), py_int(40));
    d.insert("ALERT_DESCRIPTION_BAD_CERTIFICATE".to_string(), py_int(42));
    d.insert("ALERT_DESCRIPTION_UNSUPPORTED_CERTIFICATE".to_string(), py_int(43));
    d.insert("ALERT_DESCRIPTION_CERTIFICATE_REVOKED".to_string(), py_int(44));
    d.insert("ALERT_DESCRIPTION_CERTIFICATE_EXPIRED".to_string(), py_int(45));
    d.insert("ALERT_DESCRIPTION_CERTIFICATE_UNKNOWN".to_string(), py_int(46));
    d.insert("ALERT_DESCRIPTION_INTERNAL_ERROR".to_string(), py_int(80));

    // Verify flags
    d.insert("VERIFY_DEFAULT".to_string(), py_int(0));
    d.insert("VERIFY_CRL_CHECK_LEAF".to_string(), py_int(0x10));
    d.insert("VERIFY_CRL_CHECK_CHAIN".to_string(), py_int(0x20));
    d.insert("VERIFY_X509_STRICT".to_string(), py_int(0x20));

    // Error constants
    d.insert("SSL_ERROR_ZERO_RETURN".to_string(), py_int(0));
    d.insert("SSL_ERROR_WANT_READ".to_string(), py_int(1));
    d.insert("SSL_ERROR_WANT_WRITE".to_string(), py_int(2));
    d.insert("SSL_ERROR_WANT_X509_LOOKUP".to_string(), py_int(3));
    d.insert("SSL_ERROR_SYSCALL".to_string(), py_int(5));
    d.insert("SSL_ERROR_SSL".to_string(), py_int(6));
    d.insert("SSL_ERROR_WANT_CONNECT".to_string(), py_int(7));
    d.insert("SSL_ERROR_EOF".to_string(), py_int(8));
    d.insert("SSL_ERROR_INVALID_ERROR_CODE".to_string(), py_int(20));

    // wrap_socket function — returns the socket as-is
    ssl_func!("wrap_socket", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("wrap_socket() missing required argument: sock"));
        }
        Ok(args[0].clone())
    });

    // get_default_verify_paths — stub
    ssl_func!("get_default_verify_paths", |_| {
        let mut p = HashMap::new();
        p.insert("openssl_cafile".to_string(), py_str("/etc/ssl/certs/ca-certificates.crt"));
        p.insert("openssl_capath".to_string(), py_str("/etc/ssl/certs"));
        p.insert("ssl_default_verify_paths".to_string(), py_str("(stub)"));
        Ok(create_module("_VerifyPaths", p))
    });

    // SSLContext stub — returns a module-like object with wrap_socket and other methods
    d.insert("SSLContext".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "SSLContext".to_string(),
        func: |_args| {
            let mut ctx_dict = HashMap::new();

            ctx_dict.insert("wrap_socket".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
                name: "wrap_socket".to_string(),
                func: |wargs| {
                    if wargs.is_empty() {
                        return Err(PyError::type_error("wrap_socket() missing required argument: sock"));
                    }
                    Ok(wargs[0].clone())
                },
            }));

            ctx_dict.insert("load_default_certs".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
                name: "load_default_certs".to_string(),
                func: |_| Ok(py_none()),
            }));

            ctx_dict.insert("load_verify_locations".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
                name: "load_verify_locations".to_string(),
                func: |_| Ok(py_none()),
            }));

            ctx_dict.insert("load_cert_chain".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
                name: "load_cert_chain".to_string(),
                func: |_| Ok(py_none()),
            }));

            ctx_dict.insert("set_alpn_protocols".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
                name: "set_alpn_protocols".to_string(),
                func: |_| Ok(py_none()),
            }));

            ctx_dict.insert("set_npn_protocols".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
                name: "set_npn_protocols".to_string(),
                func: |_| Ok(py_none()),
            }));

            ctx_dict.insert("set_ciphers".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
                name: "set_ciphers".to_string(),
                func: |_| Ok(py_none()),
            }));

            ctx_dict.insert("set_servername_callback".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
                name: "set_servername_callback".to_string(),
                func: |_| Ok(py_none()),
            }));

            ctx_dict.insert("get_ca_certs".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
                name: "get_ca_certs".to_string(),
                func: |_| Ok(py_list(vec![])),
            }));

            ctx_dict.insert("cert_store_stats".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
                name: "cert_store_stats".to_string(),
                func: |_| {
                    let mut s = HashMap::new();
                    s.insert("x509_ca".to_string(), py_int(0));
                    s.insert("crl".to_string(), py_int(0));
                    s.insert("x509".to_string(), py_int(0));
                    Ok(create_module("_CertStoreStats", s))
                },
            }));

            ctx_dict.insert("check_hostname".to_string(), py_bool(false));
            ctx_dict.insert("verify_mode".to_string(), py_int(0));

            Ok(create_module("SSLContext", ctx_dict))
        },
    }));

    // SSLSession stub (used by urllib3)
    ssl_func!("SSLSession", |_| Ok(py_none()));

    // CertificateError exception
    d.insert("CertificateError".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "CertificateError".to_string(),
        func: |args| {
            Ok(PyObjectRef::new(PyObject::Exception {
                typ: "CertificateError".to_string(),
                args: args.to_vec(),
                cause: None,
            }))
        },
    }));

    // SSLError exception
    d.insert("SSLError".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "SSLError".to_string(),
        func: |args| {
            Ok(PyObjectRef::new(PyObject::Exception {
                typ: "SSLError".to_string(),
                args: args.to_vec(),
                cause: None,
            }))
        },
    }));

    ssl_func!("SSLWantReadError", |args| {
        Ok(PyObjectRef::new(PyObject::Exception {
            typ: "SSLWantReadError".to_string(),
            args: args.to_vec(),
            cause: None,
        }))
    });

    ssl_func!("SSLWantWriteError", |args| {
        Ok(PyObjectRef::new(PyObject::Exception {
            typ: "SSLWantWriteError".to_string(),
            args: args.to_vec(),
            cause: None,
        }))
    });

    ssl_func!("SSLSyscallError", |args| {
        Ok(PyObjectRef::new(PyObject::Exception {
            typ: "SSLSyscallError".to_string(),
            args: args.to_vec(),
            cause: None,
        }))
    });

    ssl_func!("SSLEOFError", |args| {
        Ok(PyObjectRef::new(PyObject::Exception {
            typ: "SSLEOFError".to_string(),
            args: args.to_vec(),
            cause: None,
        }))
    });

    d.insert("__name__".to_string(), py_str("ssl"));
    d.insert("__doc__".to_string(), py_str("TLS/SSL wrapper for socket objects (stub)"));

    d
}

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
        map.get(name).and_then(|stack| stack.last().map(|(_, v)| v.clone()))
    })
}

pub fn create_contextvars_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();

    // ---- ContextVar type ----
    let mut contextvar_type_dict: HashMap<String, PyObjectRef> = HashMap::new();

    macro_rules! cv_method {
        ($name:expr, $func:expr) => {
            contextvar_type_dict.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
                name: $name.to_string(),
                func: $func,
            }));
        };
    }

    // __init__(self, name, default=None)
    cv_method!("__init__", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("ContextVar() requires at least 1 argument (name)"));
        }
        let name = args[1].str();
        if let PyObject::Instance { dict, .. } = &mut *args[0].borrow_mut() {
            dict.insert("_name".to_string(), py_str(&name));
            let default = if args.len() > 2 { args[2].clone() } else { py_none() };
            dict.insert("_default".to_string(), default);
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
                    if let Some(name_val) = dict.get("_name") {
                        return Ok(name_val.clone());
                    }
                }
                Err(PyError::type_error("ContextVar instance has no _name"))
            },
        });
        contextvar_type_dict.insert("name".to_string(), PyObjectRef::new(PyObject::Property {
            getter: Some(getter),
            setter: None,
            deleter: None,
            doc: None,
        }));
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
                let nm = dict.get("_name").ok_or_else(|| PyError::type_error("ContextVar instance has no _name"))?.str();
                let df = dict.get("_default").cloned().unwrap_or(py_none());
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
                    Err(PyError::key_error(format!("ContextVar '{}' has no value and no default", name)))
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
                dict.get("_name").ok_or_else(|| PyError::type_error("ContextVar instance has no _name"))?.str()
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
        let mut token_dict = HashMap::new();
        token_dict.insert("_token_id".to_string(), py_int(token_id as i64));
        token_dict.insert("_var_name".to_string(), py_str(&name));
        let token = PyObjectRef::new(PyObject::Instance {
            typ: TOKEN_TYPE.with(|cell| cell.borrow().clone()).ok_or_else(|| PyError::runtime_error("Token type not initialized".to_string()))?,
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
                dict.get("_token_id").and_then(|v| v.as_i64()).unwrap_or(-1) as u64
            } else {
                return Err(PyError::type_error("reset() argument must be a Token"));
            }
        };

        // Extract the variable name
        let name = {
            let borrowed = instance.borrow();
            if let PyObject::Instance { dict, .. } = &*borrowed {
                dict.get("_name").ok_or_else(|| PyError::type_error("ContextVar instance has no _name"))?.str()
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
        dict: contextvar_type_dict,
        bases: vec![],
        mro: vec![],
    });

    // ---- Token type ----
    let token_type = PyObjectRef::new(PyObject::Type {
        name: "Token".to_string(),
        dict: {
            let mut td = HashMap::new();
            // __repr__ for debugging
            td.insert("__repr__".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
                name: "__repr__".to_string(),
                func: |args| {
                    if args.is_empty() {
                        return Ok(py_str("<Token>"));
                    }
                    let borrowed = args[0].borrow();
                    if let PyObject::Instance { dict, .. } = &*borrowed {
                        if let Some(tid) = dict.get("_token_id") {
                            return Ok(py_str(&format!("<Token var={:?} id={}>",
                                dict.get("_var_name").map(|v| v.str()).unwrap_or_default(),
                                tid.as_i64().unwrap_or(-1))));
                        }
                    }
                    Ok(py_str("<Token>"))
                },
            }));
            td.insert("__name__".to_string(), py_str("Token"));
            td
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
            ctx_module_dict.insert("__name__".to_string(), py_str("Context"));

            // Add items() method using Closure so we can capture context_vals
            let items_vals = context_vals.clone();
            ctx_module_dict.insert("items".to_string(), PyObjectRef::new(PyObject::Closure(Rc::new(move |_args| {
                let mut items = Vec::new();
                for (k, v) in items_vals.iter() {
                    items.push(py_tuple(vec![py_str(k), v.clone()]));
                }
                Ok(py_list(items))
            }))));

            Ok(PyObjectRef::new(PyObject::Module {
                name: "Context".to_string(),
                dict: ctx_module_dict,
            }))
        },
    });

    // ---- Module contents ----
    d.insert("ContextVar".to_string(), contextvar_type);
    d.insert("Token".to_string(), token_type);
    d.insert("copy_context".to_string(), copy_context_func);
    d.insert("__name__".to_string(), py_str("contextvars"));
    d.insert("__doc__".to_string(), py_str("Context Variables (thread-local stub)"));

    d
}

