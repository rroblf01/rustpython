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

pub fn create_pickle_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! pickle_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    pickle_func!("dumps", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("dumps() missing required argument"));
        }
        Ok(py_str(&args[0].repr()))
    });

    pickle_func!("loads", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("loads() missing required argument"));
        }
        let s = args[0].str();
        // Evaluate the string expression using the same approach as builtin_eval
        let mut parser = crate::parser::Parser::new(&s);
        let program = parser.parse_program()
            .map_err(|e| PyError::type_error(format!("pickle.loads parse error: {}", e)))?;
        let mut compiler = crate::compiler::Compiler::new();
        let code = compiler.compile(&program, "<pickle>")
            .map_err(|e| PyError::type_error(format!("pickle.loads compile error: {}", e)))?;
        let mut vm = crate::vm::VirtualMachine::new();
        let result = vm.run(code)
            .map_err(|e| PyError::type_error(format!("pickle.loads error: {}", e)))?;
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

    // Add level constants
    d.insert("CRITICAL".to_string(), py_int(50));
    d.insert("ERROR".to_string(), py_int(40));
    d.insert("WARNING".to_string(), py_int(30));
    d.insert("INFO".to_string(), py_int(20));
    d.insert("DEBUG".to_string(), py_int(10));
    d.insert("NOTSET".to_string(), py_int(0));

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
                if let PyObject::Dict(ref d) = *dict_obj.borrow() {
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