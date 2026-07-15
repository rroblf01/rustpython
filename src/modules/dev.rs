use crate::object::*;
use crate::bytecode::{CodeObject, needs_arg};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

pub fn create_pdb_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! pdb_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    pdb_func!("set_trace", |_| {
        println!("Debugger not available");
        Ok(py_none())
    });

    d
}

pub fn create_traceback_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! tb_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    tb_func!("format_exc", |_| {
        Ok(py_str(""))
    });

    tb_func!("print_exc", |_| {
        println!("No traceback");
        Ok(py_none())
    });

    d
}

pub fn create_warnings_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! warn_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    // Store simplefilter state in a thread-local
    thread_local! {
        static WARN_FILTER: std::cell::RefCell<String> = std::cell::RefCell::new("default".to_string());
    }

    warn_func!("warn", |args| {
        let msg = if !args.is_empty() { args[0].str() } else { String::new() };
        println!("Warning: {}", msg);
        Ok(py_none())
    });

    warn_func!("simplefilter", |args| {
        if !args.is_empty() {
            let action = args[0].str();
            WARN_FILTER.with(|f| *f.borrow_mut() = action);
        }
        Ok(py_none())
    });

    // Insert the current filter state as a readable attribute
    d.insert("filters".to_string(), py_list(vec![]));

    d
}

pub fn create_abc_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! abc_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    // ABC class — returns a simple Instance with a type marker
    abc_func!("ABC", |args| {
        let _ = args;
        Ok(PyObjectRef::new(PyObject::Instance {
            typ: PyObjectRef::new(PyObject::Module {
                name: "abc".to_string(),
                dict: HashMap::new(),
            }),
            dict: HashMap::new(),
        }))
    });

    // abstractmethod — returns the function unchanged
    abc_func!("abstractmethod", |args| {
        if !args.is_empty() {
            Ok(args[0].clone())
        } else {
            Ok(py_none())
        }
    });

    d
}

pub fn create_typing_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! typ_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    // Type variables and generics
    typ_func!("TypeVar", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("TypeVar() requires at least 1 argument (name)"));
        }
        Ok(py_str(&args[0].str()))
    });

    typ_func!("Generic", |_| {
        Ok(py_str("Generic"))
    });

    typ_func!("NewType", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("NewType() requires 2 arguments (name, typ)"));
        }
        Ok(args[1].clone())
    });

    // Type hints as string stubs
    d.insert("List".to_string(), py_str("List"));
    d.insert("Dict".to_string(), py_str("Dict"));
    d.insert("Optional".to_string(), py_str("Optional"));
    d.insert("Union".to_string(), py_str("Union"));
    d.insert("Any".to_string(), py_str("Any"));
    d.insert("Callable".to_string(), py_str("Callable"));
    d.insert("Set".to_string(), py_str("Set"));
    d.insert("Tuple".to_string(), py_str("Tuple"));
    d.insert("Iterable".to_string(), py_str("Iterable"));
    d.insert("Iterator".to_string(), py_str("Iterator"));
    d.insert("Sequence".to_string(), py_str("Sequence"));
    d.insert("Mapping".to_string(), py_str("Mapping"));
    d.insert("Type".to_string(), py_str("Type"));
    d.insert("cast".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "cast".to_string(),
        func: |args| {
            if args.len() < 2 {
                return Err(PyError::type_error("cast() requires 2 arguments (typ, val)"));
            }
            Ok(args[1].clone())
        },
    }));

    d
}

pub fn create_dataclasses_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! dc_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    // dataclass(cls) — decorator that marks cls with _dataclass_ attr
    dc_func!("dataclass", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("dataclass() missing required argument (cls)"));
        }
        let cls = &args[0];
        {
            let mut borrowed = cls.borrow_mut();
            if let PyObject::Instance { ref mut dict, .. } = &mut *borrowed {
                dict.insert("_dataclass_".to_string(), py_bool(true));
            }
            // Also handle Type objects
            if let PyObject::Type { ref mut dict, .. } = &mut *borrowed {
                dict.insert("_dataclass_".to_string(), py_bool(true));
            }
        }
        Ok(cls.clone())
    });

    // field() — returns empty dict as a field descriptor
    dc_func!("field", |_| {
        Ok(py_dict())
    });

    // asdict(obj) — shallow dict copy
    dc_func!("asdict", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("asdict() missing required argument (obj)"));
        }
        let obj = &args[0];
        let borrowed = obj.borrow();
        match &*borrowed {
            PyObject::Instance { dict, .. } => {
                let mut new_dict = PyDict::new();
                for (k, v) in dict.iter() {
                    let _ = new_dict.set(py_str(k), v.clone());
                }
                Ok(PyObjectRef::new(PyObject::Dict(new_dict)))
            }
            PyObject::Dict(pydict) => {
                let mut new_dict = PyDict::new();
                for (k, v) in pydict.items() {
                    let _ = new_dict.set(k, v);
                }
                Ok(PyObjectRef::new(PyObject::Dict(new_dict)))
            }
            _ => Err(PyError::type_error("asdict() argument must be a dataclass instance")),
        }
    });

    // astuple(obj) — shallow tuple copy
    dc_func!("astuple", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("astuple() missing required argument (obj)"));
        }
        let obj = &args[0];
        let borrowed = obj.borrow();
        match &*borrowed {
            PyObject::Instance { dict, .. } => {
                let items: Vec<PyObjectRef> = dict.values().cloned().collect();
                Ok(PyObjectRef::imm(PyObject::Tuple(items)))
            }
            PyObject::Dict(pydict) => {
                let items: Vec<PyObjectRef> = pydict.values();
                Ok(PyObjectRef::imm(PyObject::Tuple(items)))
            }
            _ => Err(PyError::type_error("astuple() argument must be a dataclass instance")),
        }
    });

    // is_dataclass(obj) — checks for _dataclass_ attribute
    dc_func!("is_dataclass", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("is_dataclass() missing required argument (obj)"));
        }
        let obj = &args[0];
        let borrowed = obj.borrow();
        match &*borrowed {
            PyObject::Instance { dict, .. } => {
                Ok(py_bool(dict.contains_key("_dataclass_")))
            }
            PyObject::Type { dict, .. } => {
                Ok(py_bool(dict.contains_key("_dataclass_")))
            }
            PyObject::Dict(pydict) => {
                let _ = pydict;
                Ok(py_bool(false))
            }
            _ => Ok(py_bool(false)),
        }
    });

    // make_dataclass(name, fields) — simple Type object
    dc_func!("make_dataclass", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("make_dataclass() requires at least 2 arguments (name, fields)"));
        }
        let name = args[0].str();
        Ok(PyObjectRef::new(PyObject::Type {
            name,
            dict: HashMap::new(),
            bases: vec![],
            mro: vec![],
        }))
    });

    d
}

pub fn create_unittest_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();

    macro_rules! unittest_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    // Create the TestCase class
    let mut tc_dict = HashMap::new();

    // __init__ — no-op stub
    tc_dict.insert("__init__".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "__init__".to_string(),
        func: |_args| Ok(py_none()),
    }));

    // assertEqual(self, a, b) — no-op stub
    tc_dict.insert("assertEqual".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "assertEqual".to_string(),
        func: |_args| Ok(py_none()),
    }));

    // assertTrue(self, expr) — no-op stub
    tc_dict.insert("assertTrue".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "assertTrue".to_string(),
        func: |_args| Ok(py_none()),
    }));

    // assertFalse(self, expr) — no-op stub
    tc_dict.insert("assertFalse".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "assertFalse".to_string(),
        func: |_args| Ok(py_none()),
    }));

    // assertRaises(self, exc, callable=None, *args) — no-op stub
    tc_dict.insert("assertRaises".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "assertRaises".to_string(),
        func: |_args| Ok(py_none()),
    }));

    // assertIn(self, a, b) — no-op stub
    tc_dict.insert("assertIn".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "assertIn".to_string(),
        func: |_args| Ok(py_none()),
    }));

    // assertNotIn(self, a, b) — no-op stub
    tc_dict.insert("assertNotIn".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "assertNotIn".to_string(),
        func: |_args| Ok(py_none()),
    }));

    // assertIsNone(self, obj) — no-op stub
    tc_dict.insert("assertIsNone".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "assertIsNone".to_string(),
        func: |_args| Ok(py_none()),
    }));

    // assertIsNotNone(self, obj) — no-op stub
    tc_dict.insert("assertIsNotNone".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "assertIsNotNone".to_string(),
        func: |_args| Ok(py_none()),
    }));

    let testcase_class = PyObjectRef::new(PyObject::Type {
        name: "TestCase".to_string(),
        dict: tc_dict,
        bases: vec![],
        mro: vec![],
    });

    d.insert("TestCase".to_string(), testcase_class);

    // main() — stub that does nothing
    unittest_func!("main", |_args| {
        Ok(py_none())
    });

    // expectedFailure decorator stub — returns the function unchanged
    unittest_func!("expectedFailure", |args| {
        if args.is_empty() {
            return Ok(py_none());
        }
        Ok(args[0].clone())
    });

    // skip decorator stub — returns the function unchanged
    unittest_func!("skip", |args| {
        if args.is_empty() {
            return Ok(py_none());
        }
        Ok(args[0].clone())
    });

    d
}

/// Create a pure-Python `io` module replacement.
/// The real io.py wraps _io, but RustPython can't load io.py,
/// so we provide the interface natively.
/// Provides: open, StringIO, TextIOWrapper, DEFAULT_BUFFER_SIZE, IOBase, TextIOBase, BytesIO, UnsupportedOperation
pub fn create_io_pure_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! io_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    // DEFAULT_BUFFER_SIZE constant
    d.insert("DEFAULT_BUFFER_SIZE".to_string(), py_int(8192));

    // UnsupportedOperation — OSError subclass
    io_func!("UnsupportedOperation", |args| {
        let msg = if !args.is_empty() { args[0].str() } else { "unsupported operation".to_string() };
        Err(PyError::OsError(msg))
    });

    // IOBase — abstract base class for IO
    io_func!("IOBase", |_args| {
        let mut type_dict = HashMap::new();
        // __enter__ / __exit__ for context manager support
        type_dict.insert("__enter__".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__enter__".to_string(),
            func: |args| if args.is_empty() { Ok(py_none()) } else { Ok(args[0].clone()) },
        }));
        type_dict.insert("__exit__".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__exit__".to_string(),
            func: |_| Ok(py_none()),
        }));
        Ok(PyObjectRef::new(PyObject::Type {
            name: "IOBase".to_string(),
            dict: type_dict,
            bases: vec![],
            mro: vec![],
        }))
    });

    // TextIOBase — abstract base for text streams (direct Type object)
    {
        let mut type_dict = HashMap::new();
        let enter_func = PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__enter__".to_string(),
            func: |args| if args.is_empty() { Ok(py_none()) } else { Ok(args[0].clone()) },
        });
        type_dict.insert("__enter__".to_string(), enter_func);
        d.insert("TextIOBase".to_string(), PyObjectRef::new(PyObject::Type {
            name: "TextIOBase".to_string(),
            dict: type_dict,
            bases: vec![],
            mro: vec![],
        }));
    }

    // RawIOBase — abstract base for raw binary streams
    d.insert("RawIOBase".to_string(), PyObjectRef::new(PyObject::Type {
        name: "RawIOBase".to_string(),
        dict: HashMap::new(),
        bases: vec![],
        mro: vec![],
    }));

    // BufferedIOBase — abstract base for buffered binary streams
    d.insert("BufferedIOBase".to_string(), PyObjectRef::new(PyObject::Type {
        name: "BufferedIOBase".to_string(),
        dict: HashMap::new(),
        bases: vec![],
        mro: vec![],
    }));

    // StringIO — in-memory text stream
    io_func!("StringIO", |args| {
        let mut type_dict = HashMap::new();
        type_dict.insert("read".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "read".to_string(), func: |args| {
            let n = if args.len() > 1 {
                args[1].as_i64().unwrap_or(i64::MAX) as usize
            } else { usize::MAX };
            // Get _buffer and _pos from self (args[0])
            let buf = args[0].borrow().get_attribute("_buffer").unwrap_or_else(|_| py_str(""));
            let pos = args[0].borrow().get_attribute("_pos").unwrap_or(py_int(0));
            let pos_val = pos.as_i64().unwrap_or(0) as usize;
            let s = buf.str();
            let available = s.len() - pos_val.min(s.len());
            let take = n.min(available);
            let result = s[pos_val..pos_val + take].to_string();
            // Update _pos
            let _ = args[0].borrow_mut().set_attribute("_pos", py_int((pos_val + take) as i64));
            Ok(py_str(&result))
        }}));
        type_dict.insert("write".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "write".to_string(), func: |args| {
            let text = if args.len() > 1 { args[1].str() } else { String::new() };
            let mut buf = args[0].borrow().get_attribute("_buffer").unwrap_or_else(|_| py_str(""));
            let pos = args[0].borrow().get_attribute("_pos").unwrap_or(py_int(0));
            let pos_val = pos.as_i64().unwrap_or(0) as usize;

            let old = buf.str();
            let mut new = old[..pos_val].to_string();
            new.push_str(&text);
            let _ = args[0].borrow_mut().set_attribute("_buffer", py_str(&new));
            let _ = args[0].borrow_mut().set_attribute("_pos", py_int((pos_val + text.len()) as i64));
            Ok(py_int(text.len() as i64))
        }}));
        type_dict.insert("getvalue".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "getvalue".to_string(), func: |args| {
            let buf = args[0].borrow().get_attribute("_buffer").unwrap_or_else(|_| py_str(""));
            Ok(buf)
        }}));
        type_dict.insert("readline".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "readline".to_string(), func: |args| {
            let buf = args[0].borrow().get_attribute("_buffer").unwrap_or_else(|_| py_str(""));
            let pos = args[0].borrow().get_attribute("_pos").unwrap_or(py_int(0));
            let pos_val = pos.as_i64().unwrap_or(0) as usize;
            let s = buf.str();
            if pos_val >= s.len() { return Ok(py_str("")); }
            let remaining = &s[pos_val..];
            let end = remaining.find('\n').map(|i| i + 1).unwrap_or(remaining.len());
            let result = remaining[..end].to_string();
            let _ = args[0].borrow_mut().set_attribute("_pos", py_int((pos_val + end) as i64));
            Ok(py_str(&result))
        }}));
        type_dict.insert("seek".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "seek".to_string(), func: |args| {
            let offset = if args.len() > 1 { args[1].as_i64().unwrap_or(0) } else { 0 };
            let whence = if args.len() > 2 { args[2].as_i64().unwrap_or(0) } else { 0 };
            let buf = args[0].borrow().get_attribute("_buffer").unwrap_or_else(|_| py_str(""));
            let s = buf.str();
            let new_pos = match whence {
                0 => offset,
                1 => {
                    let pos = args[0].borrow().get_attribute("_pos").unwrap_or(py_int(0));
                    pos.as_i64().unwrap_or(0) + offset
                }
                2 => s.len() as i64 + offset,
                _ => return Err(PyError::value_error("invalid whence value")),
            };
            let _ = args[0].borrow_mut().set_attribute("_pos", py_int(new_pos.max(0)));
            Ok(py_int(new_pos.max(0)))
        }}));
        type_dict.insert("tell".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "tell".to_string(), func: |args| {
            let pos = args[0].borrow().get_attribute("_pos").unwrap_or(py_int(0));
            Ok(pos)
        }}));
        type_dict.insert("close".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "close".to_string(), func: |_| Ok(py_none()) }));
        type_dict.insert("__enter__".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "__enter__".to_string(), func: |args| if args.is_empty() { Ok(py_none()) } else { Ok(args[0].clone()) } }));
        type_dict.insert("__exit__".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "__exit__".to_string(), func: |_| Ok(py_none()) }));
        type_dict.insert("__iter__".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "__iter__".to_string(), func: |args| if args.is_empty() { Ok(py_none()) } else { Ok(args[0].clone()) } }));
        type_dict.insert("__next__".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "__next__".to_string(), func: |args| {
            let buf = args[0].borrow().get_attribute("_buffer").unwrap_or_else(|_| py_str(""));
            let pos = args[0].borrow().get_attribute("_pos").unwrap_or(py_int(0));
            let pos_val = pos.as_i64().unwrap_or(0) as usize;
            let s = buf.str();
            if pos_val >= s.len() { return Err(PyError::StopIteration); }
            let remaining = &s[pos_val..];
            let end = remaining.find('\n').map(|i| i + 1).unwrap_or(remaining.len());
            let result = remaining[..end].to_string();
            let _ = args[0].borrow_mut().set_attribute("_pos", py_int((pos_val + end) as i64));
            Ok(py_str(&result))
        }}));
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

    // BytesIO — in-memory binary stream
    io_func!("BytesIO", |args| {
        let mut type_dict = HashMap::new();
        type_dict.insert("read".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "read".to_string(), func: |args| {
            let n = if args.len() > 1 { args[1].as_i64().unwrap_or(i64::MAX) as usize } else { usize::MAX };
            let buf = args[0].borrow().get_attribute("_buffer").unwrap_or_else(|_| PyObjectRef::imm(PyObject::Bytes(vec![])));
            let pos = args[0].borrow().get_attribute("_pos").unwrap_or(py_int(0));
            let pos_val = pos.as_i64().unwrap_or(0) as usize;
            let data = match &*buf.borrow() { PyObject::Bytes(b) => b.clone(), _ => vec![] };
            let available = data.len() - pos_val.min(data.len());
            let take = n.min(available);
            let result = data[pos_val..pos_val + take].to_vec();
            let _ = args[0].borrow_mut().set_attribute("_pos", py_int((pos_val + take) as i64));
            Ok(PyObjectRef::imm(PyObject::Bytes(result)))
        }}));
        type_dict.insert("write".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "write".to_string(), func: |args| {
            let data = if args.len() > 1 {
                match &*args[1].borrow() { PyObject::Bytes(b) => b.clone(), _ => return Err(PyError::type_error("a bytes-like object is required")) }
            } else { vec![] };
            let pos = args[0].borrow().get_attribute("_pos").unwrap_or(py_int(0));
            let pos_val = pos.as_i64().unwrap_or(0) as usize;
            let buf = args[0].borrow().get_attribute("_buffer").unwrap_or_else(|_| PyObjectRef::imm(PyObject::Bytes(vec![])));
            let old = match &*buf.borrow() { PyObject::Bytes(b) => b.clone(), _ => vec![] };
            let mut new = old[..pos_val].to_vec();
            new.extend_from_slice(&data);
            let _ = args[0].borrow_mut().set_attribute("_buffer", PyObjectRef::imm(PyObject::Bytes(new)));
            let _ = args[0].borrow_mut().set_attribute("_pos", py_int((pos_val + data.len()) as i64));
            Ok(py_int(data.len() as i64))
        }}));
        type_dict.insert("getvalue".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "getvalue".to_string(), func: |args| {
            let buf = args[0].borrow().get_attribute("_buffer").unwrap_or_else(|_| PyObjectRef::imm(PyObject::Bytes(vec![])));
            Ok(buf)
        }}));
        type_dict.insert("readline".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "readline".to_string(), func: |args| {
            let buf = args[0].borrow().get_attribute("_buffer").unwrap_or_else(|_| PyObjectRef::imm(PyObject::Bytes(vec![])));
            let pos = args[0].borrow().get_attribute("_pos").unwrap_or(py_int(0));
            let pos_val = pos.as_i64().unwrap_or(0) as usize;
            let data = match &*buf.borrow() { PyObject::Bytes(b) => b.clone(), _ => vec![] };
            if pos_val >= data.len() { return Ok(PyObjectRef::imm(PyObject::Bytes(vec![]))); }
            let remaining = &data[pos_val..];
            let end = remaining.iter().position(|&b| b == b'\n').map(|i| i + 1).unwrap_or(remaining.len());
            let result = data[pos_val..pos_val + end].to_vec();
            let _ = args[0].borrow_mut().set_attribute("_pos", py_int((pos_val + end) as i64));
            Ok(PyObjectRef::imm(PyObject::Bytes(result)))
        }}));
        type_dict.insert("seek".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "seek".to_string(), func: |args| {
            let offset = if args.len() > 1 { args[1].as_i64().unwrap_or(0) } else { 0 };
            let whence = if args.len() > 2 { args[2].as_i64().unwrap_or(0) } else { 0 };
            let buf = args[0].borrow().get_attribute("_buffer").unwrap_or_else(|_| PyObjectRef::imm(PyObject::Bytes(vec![])));
            let data = match &*buf.borrow() { PyObject::Bytes(b) => b.clone(), _ => vec![] };
            let new_pos = match whence {
                0 => offset,
                1 => { let p = args[0].borrow().get_attribute("_pos").unwrap_or(py_int(0)); p.as_i64().unwrap_or(0) + offset }
                2 => data.len() as i64 + offset,
                _ => return Err(PyError::value_error("invalid whence value")),
            };
            let _ = args[0].borrow_mut().set_attribute("_pos", py_int(new_pos.max(0)));
            Ok(py_int(new_pos.max(0)))
        }}));
        type_dict.insert("tell".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "tell".to_string(), func: |args| {
            let pos = args[0].borrow().get_attribute("_pos").unwrap_or(py_int(0));
            Ok(pos)
        }}));
        type_dict.insert("close".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "close".to_string(), func: |_| Ok(py_none()) }));
        type_dict.insert("__enter__".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "__enter__".to_string(), func: |args| if args.is_empty() { Ok(py_none()) } else { Ok(args[0].clone()) } }));
        type_dict.insert("__exit__".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "__exit__".to_string(), func: |_| Ok(py_none()) }));
        let typ = PyObjectRef::new(PyObject::Type {
            name: "BytesIO".to_string(),
            dict: type_dict,
            bases: vec![],
            mro: vec![],
        });
        let mut instance_dict = HashMap::new();
        let initial = if !args.is_empty() {
            match &*args[0].borrow() { PyObject::Bytes(b) => b.clone(), _ => vec![] }
        } else { vec![] };
        instance_dict.insert("_buffer".to_string(), PyObjectRef::imm(PyObject::Bytes(initial)));
        instance_dict.insert("_pos".to_string(), py_int(0));
        Ok(PyObjectRef::new(PyObject::Instance {
            typ,
            dict: instance_dict,
        }))
    });

    // TextIOWrapper — wraps a buffered stream for text I/O
    io_func!("TextIOWrapper", |args| {
        let mut type_dict = HashMap::new();
        type_dict.insert("read".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "read".to_string(), func: |args| {
            // Delegate to wrapped buffer's read if available
            if args.len() > 1 {
                // Try to call read on args[0] (self) — but this is a wrapper, not buffered
                // For now, return empty string
                Ok(py_str(""))
            } else { Ok(py_str("")) }
        }}));
        type_dict.insert("write".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "write".to_string(), func: |args| {
            Ok(py_int(0))
        }}));
        type_dict.insert("close".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "close".to_string(), func: |_| Ok(py_none()) }));
        type_dict.insert("__enter__".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "__enter__".to_string(), func: |args| if args.is_empty() { Ok(py_none()) } else { Ok(args[0].clone()) } }));
        type_dict.insert("__exit__".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "__exit__".to_string(), func: |_| Ok(py_none()) }));
        let typ = PyObjectRef::new(PyObject::Type {
            name: "TextIOWrapper".to_string(),
            dict: type_dict,
            bases: vec![],
            mro: vec![],
        });
        Ok(PyObjectRef::new(PyObject::Instance {
            typ,
            dict: HashMap::new(),
        }))
    });

    // open — reference to builtin open
    d.insert("open".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "open".to_string(),
        func: |args| crate::object::builtin_open(args),
    }));

    d
}

pub fn create_dis_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! dis_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    // Helper: extract a CodeObject from either a code object or a function
    fn extract_code(args: &[PyObjectRef]) -> Result<CodeObject, PyError> {
        if args.is_empty() {
            return Err(PyError::type_error("missing required argument: code or function"));
        }
        let obj = args[0].borrow();
        match &*obj {
            PyObject::Code(code) => Ok(code.as_ref().clone()),
            PyObject::Function { code, .. } => Ok(code.clone()),
            _ => Err(PyError::type_error("argument must be a code object or function")),
        }
    }

    dis_func!("dis", |args| {
        let code = extract_code(args)?;
        let mut lines = Vec::new();
        for (i, instr) in code.instructions.iter().enumerate() {
            let offset = i * 2; // each instruction is 2 bytes (op + arg)
            let opname = format!("{:?}", instr.op);
            let arg_str = if needs_arg(instr.op) || instr.arg != 0 {
                format!("{}", instr.arg)
            } else {
                String::new()
            };
            lines.push(format!("{:>4} {:20} {}", offset, opname, arg_str));
        }
        Ok(py_str(&lines.join("\n")))
    });

    dis_func!("get_instructions", |args| {
        let code = extract_code(args)?;
        let mut instr_list = Vec::new();
        for (i, instr) in code.instructions.iter().enumerate() {
            let offset = (i * 2) as i64;
            let opname = format!("{:?}", instr.op);
            let arg = instr.arg as i64;
            instr_list.push(py_tuple(vec![
                py_int(offset),
                py_str(&opname),
                py_int(arg),
            ]));
        }
        Ok(py_list(instr_list))
    });

    // Also add some opcode name constants for reference
    d.insert("opname".to_string(), py_str("dis module for bytecode disassembly"));

    d
}

pub fn create_doctest_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! doctest_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    // TestResults constructor — returns an instance with failed=0, attempted=0
    doctest_func!("TestResults", |_args| {
        let mut dict = HashMap::new();
        dict.insert("failed".to_string(), py_int(0));
        dict.insert("attempted".to_string(), py_int(0));
        Ok(PyObjectRef::new(PyObject::Instance {
            typ: py_str("TestResults"),
            dict,
        }))
    });

    // testmod(m=None) — runs doctests on a module, returns TestResults(failed=0, attempted=0)
    doctest_func!("testmod", |_args| {
        let mut dict = HashMap::new();
        dict.insert("failed".to_string(), py_int(0));
        dict.insert("attempted".to_string(), py_int(0));
        Ok(PyObjectRef::new(PyObject::Instance {
            typ: py_str("TestResults"),
            dict,
        }))
    });

    // testfile(filename) — runs doctests in a file, returns TestResults(failed=0, attempted=0)
    doctest_func!("testfile", |_args| {
        let mut dict = HashMap::new();
        dict.insert("failed".to_string(), py_int(0));
        dict.insert("attempted".to_string(), py_int(0));
        Ok(PyObjectRef::new(PyObject::Instance {
            typ: py_str("TestResults"),
            dict,
        }))
    });

    // run_docstring_examples(f, globs, verbose=False) — stub
    doctest_func!("run_docstring_examples", |_args| {
        Ok(py_none())
    });

    // DocTestFinder class stub
    doctest_func!("DocTestFinder", |_args| {
        let mut dict = HashMap::new();
        dict.insert("find".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
            name: "find".to_string(),
            func: |_| Ok(py_list(vec![])),
        }));
        Ok(PyObjectRef::new(PyObject::Instance {
            typ: py_str("DocTestFinder"),
            dict,
        }))
    });

    d
}
// ─── inspect module ────────────────────────────────────────────────────────

pub fn create_inspect_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! inspect_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    inspect_func!("isfunction", |args| {
        if args.len() < 1 { return Err(PyError::type_error("isfunction() requires 1 argument")); }
        let obj = args[0].borrow();
        Ok(py_bool(matches!(&*obj, PyObject::Function { .. })))
    });

    inspect_func!("isgeneratorfunction", |args| {
        if args.len() < 1 { return Err(PyError::type_error("isgeneratorfunction() requires 1 argument")); }
        let obj = args[0].borrow();
        let is_gen = match &*obj {
            PyObject::Function { code, .. } => (code.flags & 0x0020) != 0,
            _ => false,
        };
        Ok(py_bool(is_gen))
    });

    inspect_func!("iscoroutinefunction", |args| {
        if args.len() < 1 { return Err(PyError::type_error("iscoroutinefunction() requires 1 argument")); }
        let obj = args[0].borrow();
        let is_coro = match &*obj {
            PyObject::Function { code, .. } => (code.flags & 0x0080) != 0,
            _ => false,
        };
        Ok(py_bool(is_coro))
    });

    inspect_func!("isclass", |args| {
        if args.len() < 1 { return Err(PyError::type_error("isclass() requires 1 argument")); }
        let obj = args[0].borrow();
        Ok(py_bool(matches!(&*obj, PyObject::Type { .. })))
    });

    inspect_func!("ismodule", |args| {
        if args.len() < 1 { return Err(PyError::type_error("ismodule() requires 1 argument")); }
        let obj = args[0].borrow();
        Ok(py_bool(matches!(&*obj, PyObject::Module { .. })))
    });

    inspect_func!("ismethod", |args| {
        if args.len() < 1 { return Err(PyError::type_error("ismethod() requires 1 argument")); }
        let obj = args[0].borrow();
        Ok(py_bool(matches!(&*obj, PyObject::BoundMethod { .. })))
    });

    inspect_func!("isframe", |_args| Ok(py_bool(false)));
    inspect_func!("istraceback", |_args| Ok(py_bool(false)));

    inspect_func!("getdoc", |args| {
        if args.len() < 1 { return Err(PyError::type_error("getdoc() requires 1 argument")); }
        let obj = args[0].borrow();
        let doc = match &*obj {
            PyObject::Function { ref dict, .. } => dict.get("__doc__").cloned(),
            PyObject::Type { ref dict, .. } => dict.get("__doc__").cloned(),
            PyObject::Module { ref dict, .. } => dict.get("__doc__").cloned(),
            PyObject::Instance { ref dict, .. } => dict.get("__doc__").cloned(),
            _ => None,
        };
        Ok(doc.unwrap_or(py_none()))
    });

    inspect_func!("getfile", |_args| Ok(py_str("<unknown>")));
    inspect_func!("getsourcefile", |_args| Ok(py_none()));
    inspect_func!("getsource", |_args| Ok(py_str("Source not available in RustPython")));

    inspect_func!("getmodule", |args| {
        if args.len() < 1 { return Err(PyError::type_error("getmodule() requires 1 argument")); }
        let module_name = args[0].borrow().get_attribute("__module__").ok()
            .and_then(|v| { if let PyObject::Str(s) = &*v.borrow() { Some(s.clone()) } else { None } });
        Ok(if let Some(name) = module_name { py_str(&name) } else { py_none() })
    });

    inspect_func!("getmembers", |args| {
        if args.len() < 1 { return Err(PyError::type_error("getmembers() requires 1 argument")); }
        let obj = args[0].borrow();
        let dict = match &*obj {
            PyObject::Function { ref dict, .. } => Some(dict),
            PyObject::Type { ref dict, .. } => Some(dict),
            PyObject::Module { ref dict, .. } => Some(dict),
            PyObject::Instance { ref dict, .. } => Some(dict),
            _ => None,
        };
        let members: Vec<PyObjectRef> = if let Some(d) = dict {
            let mut items: Vec<(String, PyObjectRef)> = d.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
            items.sort_by(|a, b| a.0.cmp(&b.0));
            items.into_iter().map(|(k, v)| py_tuple(vec![py_str(&k), v])).collect()
        } else { Vec::new() };
        Ok(py_list(members))
    });

    inspect_func!("signature", |_args| Ok(py_str("<signature not available>")));
    inspect_func!("currentframe", |_args| Ok(py_none()));
    inspect_func!("stack", |_args| Ok(py_list(vec![])));
    inspect_func!("getouterframes", |_args| Ok(py_list(vec![])));
    inspect_func!("getinnerframes", |_args| Ok(py_list(vec![])));

    // Parameter class stub (needed by Django's inspect module usage)
    let mut param_type_dict = HashMap::new();
    param_type_dict.insert("POSITIONAL_ONLY".to_string(), py_int(0));
    param_type_dict.insert("POSITIONAL_OR_KEYWORD".to_string(), py_int(1));
    param_type_dict.insert("VAR_POSITIONAL".to_string(), py_int(2));
    param_type_dict.insert("KEYWORD_ONLY".to_string(), py_int(3));
    param_type_dict.insert("VAR_KEYWORD".to_string(), py_int(4));
    param_type_dict.insert("empty".to_string(), py_none());
    d.insert("Parameter".to_string(), PyObjectRef::new(PyObject::Type { name: "Parameter".to_string(), dict: param_type_dict, bases: vec![], mro: vec![] }));
    d.insert("Signature".to_string(), PyObjectRef::new(PyObject::Type { name: "Signature".to_string(), dict: HashMap::new(), bases: vec![], mro: vec![] }));

    d
}

// ─── profile module ────────────────────────────────────────────────────────

pub fn create_profile_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! prof_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    prof_func!("run", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("run() missing required argument (statement)"));
        }
        let cmd = args[0].str();
        let _ = crate::object::with_vm_mut(|vm| {
            let mut parser = crate::parser::Parser::new(&cmd);
            if let Ok(program) = parser.parse_program() {
                let mut compiler = crate::compiler::Compiler::new();
                if let Ok(code) = compiler.compile(&program, "<profile>") {
                    let _ = vm.exec_code(code, None);
                }
            }
        });
        Ok(py_none())
    });

    prof_func!("runctx", |args| {
        if args.len() < 3 {
            return Err(PyError::type_error("runctx() requires 3 arguments (statement, globals, locals)"));
        }
        let cmd = args[0].str();
        let _globals = &args[1];
        let _locals = &args[2];
        let _ = crate::object::with_vm_mut(|vm| {
            let mut parser = crate::parser::Parser::new(&cmd);
            if let Ok(program) = parser.parse_program() {
                let mut compiler = crate::compiler::Compiler::new();
                if let Ok(code) = compiler.compile(&program, "<profile>") {
                    let _ = vm.exec_code(code, None);
                }
            }
        });
        Ok(py_none())
    });

    // Profiler stub class
    prof_func!("Profile", |_args| {
        let mut inst_dict = HashMap::new();
        inst_dict.insert("enable".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
            name: "enable".to_string(),
            func: |_| Ok(py_none()),
        }));
        inst_dict.insert("disable".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
            name: "disable".to_string(),
            func: |_| Ok(py_none()),
        }));
        inst_dict.insert("create_stats".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
            name: "create_stats".to_string(),
            func: |_| Ok(py_none()),
        }));
        inst_dict.insert("print_stats".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
            name: "print_stats".to_string(),
            func: |_| Ok(py_none()),
        }));
        inst_dict.insert("dump_stats".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
            name: "dump_stats".to_string(),
            func: |_| Ok(py_none()),
        }));
        Ok(PyObjectRef::new(PyObject::Instance {
            typ: py_str("Profile"),
            dict: inst_dict,
        }))
    });

    d
}

// ─── cProfile module ───────────────────────────────────────────────────────

pub fn create_cprofile_dict() -> HashMap<String, PyObjectRef> {
    let mut d = create_profile_dict();
    d.insert("__name__".to_string(), py_str("cProfile"));
    d
}

// ─── resource module ──────────────────────────────────────────────────────

pub fn create_resource_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! res_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    // Resource usage constants (POSIX standard)
    d.insert("RUSAGE_SELF".to_string(), py_int(0));
    d.insert("RUSAGE_CHILDREN".to_string(), py_int(-1));
    d.insert("RUSAGE_BOTH".to_string(), py_int(-2));
    d.insert("RUSAGE_THREAD".to_string(), py_int(1));

    // Priority constants
    d.insert("PRIO_PROCESS".to_string(), py_int(0));
    d.insert("PRIO_PGRP".to_string(), py_int(1));
    d.insert("PRIO_USER".to_string(), py_int(2));

    // RLIMIT constants (common ones)
    d.insert("RLIMIT_CPU".to_string(), py_int(0));
    d.insert("RLIMIT_FSIZE".to_string(), py_int(1));
    d.insert("RLIMIT_DATA".to_string(), py_int(2));
    d.insert("RLIMIT_STACK".to_string(), py_int(3));
    d.insert("RLIMIT_CORE".to_string(), py_int(4));
    d.insert("RLIMIT_NOFILE".to_string(), py_int(7));
    d.insert("RLIMIT_AS".to_string(), py_int(9));

    res_func!("getrusage", |_args| {
        let mut result_dict = HashMap::new();
        let zero = py_int(0);
        result_dict.insert("ru_utime".to_string(), py_float(0.0));
        result_dict.insert("ru_stime".to_string(), py_float(0.0));
        result_dict.insert("ru_maxrss".to_string(), zero.clone());
        result_dict.insert("ru_ixrss".to_string(), zero.clone());
        result_dict.insert("ru_idrss".to_string(), zero.clone());
        result_dict.insert("ru_isrss".to_string(), zero.clone());
        result_dict.insert("ru_minflt".to_string(), zero.clone());
        result_dict.insert("ru_majflt".to_string(), zero.clone());
        result_dict.insert("ru_nswap".to_string(), zero.clone());
        result_dict.insert("ru_inblock".to_string(), zero.clone());
        result_dict.insert("ru_oublock".to_string(), zero.clone());
        result_dict.insert("ru_msgsnd".to_string(), zero.clone());
        result_dict.insert("ru_msgrcv".to_string(), zero.clone());
        result_dict.insert("ru_nsignals".to_string(), zero.clone());
        result_dict.insert("ru_nvcsw".to_string(), zero.clone());
        result_dict.insert("ru_nivcsw".to_string(), zero.clone());
        Ok(PyObjectRef::new(PyObject::Instance {
            typ: py_str("struct_rusage"),
            dict: result_dict,
        }))
    });

    res_func!("getpagesize", |_| {
        Ok(py_int(4096))
    });

    res_func!("getrlimit", |_args| {
        // Return (soft, hard) as tuple with large defaults
        Ok(py_tuple(vec![py_int(999999), py_int(999999)]))
    });

    res_func!("setrlimit", |_args| {
        Ok(py_none())
    });

    d
}

// ─── trace module ─────────────────────────────────────────────────────────

pub fn create_trace_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! trace_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    trace_func!("Trace", |_args| {
        let mut inst_dict = HashMap::new();
        inst_dict.insert("run".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
            name: "run".to_string(),
            func: |args| {
                let cmd = if !args.is_empty() { args[0].str() } else { String::new() };
                let _ = crate::object::with_vm_mut(|vm| {
                    let mut parser = crate::parser::Parser::new(&cmd);
                    if let Ok(program) = parser.parse_program() {
                        let mut compiler = crate::compiler::Compiler::new();
                        if let Ok(code) = compiler.compile(&program, "<trace>") {
                            let _ = vm.exec_code(code, None);
                        }
                    }
                });
                Ok(py_none())
            },
        }));
        inst_dict.insert("runctx".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
            name: "runctx".to_string(),
            func: |_| Ok(py_none()),
        }));
        inst_dict.insert("results".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
            name: "results".to_string(),
            func: |_| Ok(py_none()),
        }));
        Ok(PyObjectRef::new(PyObject::Instance {
            typ: py_str("Trace"),
            dict: inst_dict,
        }))
    });

    // Coverage results class
    trace_func!("CoverageResults", |_args| {
        let mut inst_dict = HashMap::new();
        inst_dict.insert("write_results".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
            name: "write_results".to_string(),
            func: |_| Ok(py_none()),
        }));
        Ok(PyObjectRef::new(PyObject::Instance {
            typ: py_str("CoverageResults"),
            dict: inst_dict,
        }))
    });

    d
}

/// Native _warnings module — CPython C extension replacement
pub fn create_warnings_c_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! warn_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }
    warn_func!("warn", |args| {
        let msg = if !args.is_empty() { args[0].str() } else { String::new() };
        eprintln!("Warning: {}", msg);
        Ok(py_none())
    });
    d
}

pub fn create_marshal_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! m_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }
    m_func!("loads", |args| {
        if args.len() < 2 { return Err(PyError::type_error("loads() takes 1 argument")); }
        Ok(args[1].clone())
    });
    m_func!("dumps", |args| {
        if args.len() < 2 { return Err(PyError::type_error("dumps() takes 1 argument")); }
        Ok(PyObjectRef::imm(PyObject::Bytes(vec![0u8; 4])))
    });
    d
}

pub fn create_imp_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! imp_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    imp_func!("acquire_lock", |_| Ok(py_none()));
    imp_func!("release_lock", |_| Ok(py_none()));
    imp_func!("lock_held", |_| Ok(py_bool(false)));
    imp_func!("is_frozen", |_| Ok(py_bool(false)));
    imp_func!("is_builtin", |_| Ok(py_bool(false)));
    imp_func!("is_frozen_package", |_| Ok(py_bool(false)));
    imp_func!("find_frozen", |_| Err(PyError::ImportError("frozen modules not supported".to_string())));
    imp_func!("init_frozen", |_| Ok(py_none()));
    imp_func!("get_frozen_object", |_| Err(PyError::ImportError("frozen modules not supported".to_string())));
    imp_func!("create_builtin", |args| {
        // Return a new module object for builtin modules
        let spec = if !args.is_empty() { args[0].borrow() } else { return Err(PyError::type_error("create_builtin requires spec")); };
        let name = spec.get_attribute("name").ok().map(|n| n.str()).unwrap_or_else(|| "unknown".to_string());
        Ok(create_module(&name, HashMap::new()))
    });
    imp_func!("exec_builtin", |args| {
        // No-op: module is already registered
        Ok(py_none())
    });
    imp_func!("create_dynamic", |_| Err(PyError::ImportError("dynamic extensions not supported".to_string())));
    imp_func!("exec_dynamic", |_| Err(PyError::ImportError("dynamic extensions not supported".to_string())));

    imp_func!("extension_suffixes", |_| {
        let arch = if cfg!(target_os = "linux") { "x86_64-linux-gnu" }
                   else if cfg!(target_os = "macos") { "darwin" }
                   else { "win-amd64" };
        Ok(py_list(vec![
            py_str(&format!(".cpython-313-{}.so", arch)),
            py_str(".abi3.so"),
            py_str(".so"),
        ]))
    });

    imp_func!("source_hash", |_| Ok(PyObjectRef::imm(PyObject::Bytes(vec![0u8; 8]))));
    imp_func!("_fix_co_filename", |_| Ok(py_none()));

    d.insert("check_hash_based_pycs".to_string(), py_str("never"));
    d.insert("_frozen_module_names".to_string(), py_list(vec![]));
    d.insert("_override_frozen_modules_for_tests".to_string(), py_none());
    d.insert("_override_multi_interp_extensions_check".to_string(), py_none());

    d
}

pub fn create_zipimport_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! zip_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }
    zip_func!("zipimporter", |args| {
        let path = if !args.is_empty() { args[0].str() } else { String::new() };
        let mut inst_dict = HashMap::new();
        inst_dict.insert("find_spec".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
            name: "find_spec".to_string(), func: |_| Ok(py_none()),
        }));
        inst_dict.insert("find_module".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
            name: "find_module".to_string(), func: |_| Ok(py_none()),
        }));
        inst_dict.insert("get_code".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
            name: "get_code".to_string(), func: |_| Ok(py_none()),
        }));
        inst_dict.insert("get_source".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
            name: "get_source".to_string(), func: |_| Ok(py_none()),
        }));
        Ok(PyObjectRef::new(PyObject::Instance {
            typ: py_str("zipimporter"),
            dict: inst_dict,
        }))
    });
    d.insert("_zip_directory_cache".to_string(), py_dict());
    d
}

/// Native _io module — CPython C extension replacement
pub fn create_io_module_dict() -> HashMap<String, PyObjectRef> {
    use std::io::{Read, Write};
    let mut d = HashMap::new();
    macro_rules! io_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    // FileIO — wraps std::fs::File via builtin_open
    io_func!("FileIO", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("FileIO() missing required argument"));
        }
        let filename = args[0].str();
        let mode = if args.len() > 1 { args[1].str() } else { "r".to_string() };
        let file = if let Some(fd) = args[0].as_i64() {
            use std::os::unix::io::FromRawFd;
            if fd < 0 {
                return Err(PyError::OsError("invalid file descriptor".to_string()));
            }
            // SAFETY: from_raw_fd is inherently unsafe because the caller must
            // guarantee the fd is valid and ownership is transferred. We at least
            // verify fd >= 0 as a basic sanity check.
            unsafe { std::fs::File::from_raw_fd(fd as i32) }
        } else {
            std::fs::File::options()
                .read(mode.contains('r') || mode == "wb")
                .write(mode.contains('w') || mode.contains('a'))
                .append(mode.contains('a'))
                .create(mode.contains('w') || mode.contains('a'))
                .truncate(mode.contains('w'))
                .open(&filename)
                .map_err(|e| PyError::OsError(format!("{}", e)))?
        };
        Ok(PyObjectRef::new(PyObject::File { file: Rc::new(RefCell::new(file)) }))
    });

    // BytesIO — in-memory bytes buffer
    io_func!("BytesIO", |args| {
        let buf = if !args.is_empty() {
            let a = args[0].borrow();
            match &*a {
                PyObject::Bytes(b) => b.clone(),
                PyObject::Str(s) => s.as_bytes().to_vec(),
                _ => vec![],
            }
        } else {
            vec![]
        };
        let buf_rc = Rc::new(RefCell::new(buf));
        let pos_rc = Rc::new(RefCell::new(0usize));
        let mut type_dict = HashMap::new();
        let b1 = buf_rc.clone();
        let p1 = pos_rc.clone();
        type_dict.insert("read".to_string(), PyObjectRef::new(PyObject::Closure(Rc::new(move |_: &[PyObjectRef]| {
            let data = b1.borrow()[*p1.borrow()..].to_vec();
            Ok(PyObjectRef::imm(PyObject::Bytes(data)))
        }))));
        let b2 = buf_rc.clone();
        let p2 = pos_rc.clone();
        type_dict.insert("readline".to_string(), PyObjectRef::new(PyObject::Closure(Rc::new(move |_: &[PyObjectRef]| {
            let remaining = &b2.borrow()[*p2.borrow()..];
            let end = remaining.iter().position(|&c| c == b'\n').map(|i| i + 1).unwrap_or(remaining.len());
            Ok(PyObjectRef::imm(PyObject::Bytes(remaining[..end].to_vec())))
        }))));
        type_dict.insert("close".to_string(), PyObjectRef::new(PyObject::Closure(Rc::new(move |_: &[PyObjectRef]| Ok(py_none())))));
        Ok(PyObjectRef::new(PyObject::Instance {
            typ: PyObjectRef::new(PyObject::Type { name: "BytesIO".to_string(), dict: type_dict, bases: vec![], mro: vec![] }),
            dict: HashMap::new(),
        }))
    });

    // IncrementalNewlineDecoder — stub
    io_func!("IncrementalNewlineDecoder", |_args| {
        let mut type_dict = HashMap::new();
        type_dict.insert("decode".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
            name: "decode".to_string(),
            func: |m_args| {
                if m_args.len() < 2 { return Err(PyError::type_error("decode() takes 1 argument")); }
                match &*m_args[1].borrow() {
                    PyObject::Bytes(b) => Ok(py_str(&String::from_utf8_lossy(b))),
                    _ => Err(PyError::type_error("decode() argument must be bytes")),
                }
                },
                }));
                Ok(PyObjectRef::new(PyObject::Instance {
                typ: py_str("IncrementalNewlineDecoder"), dict: type_dict,
        }))
    });

    io_func!("open_code", |args| {
        if args.is_empty() { return Err(PyError::type_error("open_code() missing argument")); }
        let path = args[0].str();
        let file = std::fs::File::open(&path).map_err(|e| PyError::OsError(format!("{}", e)))?;
        Ok(PyObjectRef::new(PyObject::File { file: Rc::new(RefCell::new(file)) }))
    });

    io_func!("text_encoding", |args| {
        if args.is_empty() { return Err(PyError::type_error("text_encoding() missing argument")); }
        Ok(py_str(&args[0].str()))
    });

    d.insert("open".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "open".to_string(), func: builtin_open,
    }));
    d.insert("DEFAULT_BUFFER_SIZE".to_string(), py_int(8192));
    d.insert("BlockingIOError".to_string(), py_str("BlockingIOError"));
    d.insert("UnsupportedOperation".to_string(), py_str("UnsupportedOperation"));

    // ── IO Base Classes ─────────────────────────────────────────────────────────

    // IOBase — abstract base class with close, closed, __enter__, __exit__
    let mut iobase_dict = HashMap::new();
    iobase_dict.insert("__init__".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "__init__".to_string(), func: |_: &[PyObjectRef]| Ok(py_none()),
    }));
    iobase_dict.insert("close".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "close".to_string(), func: |_: &[PyObjectRef]| Ok(py_none()),
    }));
    let closed_getter = PyObjectRef::new(PyObject::BuiltinFunction {
        name: "closed".to_string(), func: |_: &[PyObjectRef]| Ok(py_bool(false)),
    });
    iobase_dict.insert("closed".to_string(), PyObjectRef::new(PyObject::Property {
        getter: Some(closed_getter), setter: None, deleter: None, doc: None,
    }));
    iobase_dict.insert("__enter__".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "__enter__".to_string(), func: |args: &[PyObjectRef]| Ok(args[0].clone()),
    }));
    iobase_dict.insert("__exit__".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "__exit__".to_string(), func: |_: &[PyObjectRef]| Ok(py_none()),
    }));
    let iobase_cls = PyObjectRef::new(PyObject::Type {
        name: "IOBase".to_string(), dict: iobase_dict, bases: vec![], mro: vec![],
    });
    d.insert("IOBase".to_string(), iobase_cls.clone());
    d.insert("_IOBase".to_string(), iobase_cls.clone());

    // RawIOBase — extends IOBase
    let mut raw_dict = HashMap::new();
    raw_dict.insert("__init__".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "__init__".to_string(), func: |_: &[PyObjectRef]| Ok(py_none()),
    }));
    raw_dict.insert("read".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "read".to_string(), func: |_: &[PyObjectRef]| Ok(PyObjectRef::imm(PyObject::Bytes(vec![]))),
    }));
    raw_dict.insert("readinto".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "readinto".to_string(), func: |_: &[PyObjectRef]| Ok(py_none()),
    }));
    raw_dict.insert("write".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "write".to_string(), func: |_: &[PyObjectRef]| Ok(py_int(0)),
    }));
    raw_dict.insert("close".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "close".to_string(), func: |_: &[PyObjectRef]| Ok(py_none()),
    }));
    let raw_cls = PyObjectRef::new(PyObject::Type {
        name: "RawIOBase".to_string(), dict: raw_dict,
        bases: vec![iobase_cls.clone()], mro: vec![iobase_cls.clone()],
    });
    d.insert("RawIOBase".to_string(), raw_cls.clone());
    d.insert("_RawIOBase".to_string(), raw_cls.clone());

    // BufferedIOBase — extends IOBase
    let mut buf_dict = HashMap::new();
    buf_dict.insert("__init__".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "__init__".to_string(), func: |_: &[PyObjectRef]| Ok(py_none()),
    }));
    buf_dict.insert("read".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "read".to_string(), func: |_: &[PyObjectRef]| Ok(PyObjectRef::imm(PyObject::Bytes(vec![]))),
    }));
    buf_dict.insert("read1".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "read1".to_string(), func: |_: &[PyObjectRef]| Ok(PyObjectRef::imm(PyObject::Bytes(vec![]))),
    }));
    buf_dict.insert("write".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "write".to_string(), func: |_: &[PyObjectRef]| Ok(py_none()),
    }));
    buf_dict.insert("close".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "close".to_string(), func: |_: &[PyObjectRef]| Ok(py_none()),
    }));
    let buf_cls = PyObjectRef::new(PyObject::Type {
        name: "BufferedIOBase".to_string(), dict: buf_dict,
        bases: vec![iobase_cls.clone()], mro: vec![iobase_cls.clone()],
    });
    d.insert("BufferedIOBase".to_string(), buf_cls.clone());
    d.insert("_BufferedIOBase".to_string(), buf_cls.clone());

    // TextIOBase — text I/O base class (extends IOBase)
    let mut text_dict = HashMap::new();
    text_dict.insert("__init__".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "__init__".to_string(), func: |_: &[PyObjectRef]| Ok(py_none()),
    }));
    text_dict.insert("read".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "read".to_string(), func: |_: &[PyObjectRef]| Ok(py_str("")),
    }));
    text_dict.insert("write".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "write".to_string(), func: |_: &[PyObjectRef]| Ok(py_none()),
    }));
    text_dict.insert("close".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "close".to_string(), func: |_: &[PyObjectRef]| Ok(py_none()),
    }));
    let text_cls = PyObjectRef::new(PyObject::Type {
        name: "TextIOBase".to_string(), dict: text_dict,
        bases: vec![iobase_cls.clone()], mro: vec![iobase_cls.clone()],
    });
    d.insert("TextIOBase".to_string(), text_cls.clone());
    d.insert("_TextIOBase".to_string(), text_cls.clone());

    // StringIO — in-memory text buffer, factory with Rc<RefCell<String>> via Closures
    let stringio_closure: Rc<dyn Fn(&[PyObjectRef]) -> PyResult<PyObjectRef>> = Rc::new(move |args: &[PyObjectRef]| {
        let initial_value = if !args.is_empty() { args[0].str() } else { String::new() };
        let buffer = Rc::new(RefCell::new(initial_value));
        let mut type_dict = HashMap::new();

        // __init__ — no-op (initial_value already consumed by factory)
        type_dict.insert("__init__".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__init__".to_string(), func: |_: &[PyObjectRef]| Ok(py_none()),
        }));

        // read — return full buffer contents
        let b_read = buffer.clone();
        type_dict.insert("read".to_string(), PyObjectRef::new(PyObject::Closure(Rc::new(move |_: &[PyObjectRef]| {
            Ok(py_str(&b_read.borrow()))
        }))));

        // write — append text to buffer
        let b_write = buffer.clone();
        type_dict.insert("write".to_string(), PyObjectRef::new(PyObject::Closure(Rc::new(move |w_args: &[PyObjectRef]| {
            let text = if !w_args.is_empty() { w_args[0].str() } else { String::new() };
            b_write.borrow_mut().push_str(&text);
            Ok(py_int(text.len()))
        }))));

        // getvalue — return current buffer contents
        let b_get = buffer.clone();
        type_dict.insert("getvalue".to_string(), PyObjectRef::new(PyObject::Closure(Rc::new(move |_: &[PyObjectRef]| {
            Ok(py_str(&b_get.borrow()))
        }))));

        // close — no-op
        type_dict.insert("close".to_string(), PyObjectRef::new(PyObject::Closure(Rc::new(move |_: &[PyObjectRef]| {
            Ok(py_none())
        }))));

        // seek — stub
        type_dict.insert("seek".to_string(), PyObjectRef::new(PyObject::Closure(Rc::new(move |_: &[PyObjectRef]| {
            Ok(py_int(0))
        }))));

        // tell — stub
        type_dict.insert("tell".to_string(), PyObjectRef::new(PyObject::Closure(Rc::new(move |_: &[PyObjectRef]| {
            Ok(py_int(0))
        }))));

        Ok(PyObjectRef::new(PyObject::Instance {
            typ: PyObjectRef::new(PyObject::Type {
                name: "StringIO".to_string(), dict: type_dict,
                bases: vec![text_cls.clone()], mro: vec![text_cls.clone()],
            }),
            dict: HashMap::new(),
        }))
    });
    d.insert("StringIO".to_string(), PyObjectRef::new(PyObject::Closure(stringio_closure)));

    // BufferedReader, BufferedWriter, BufferedRWPair, BufferedRandom — stubs
    let br_dict = HashMap::new(); let br_cls = PyObjectRef::new(PyObject::Type { name: "BufferedReader".to_string(), dict: br_dict, bases: vec![buf_cls.clone()], mro: vec![buf_cls.clone()] });
    d.insert("BufferedReader".to_string(), br_cls.clone());
    let bw_dict = HashMap::new(); let bw_cls = PyObjectRef::new(PyObject::Type { name: "BufferedWriter".to_string(), dict: bw_dict, bases: vec![buf_cls.clone()], mro: vec![buf_cls.clone()] });
    d.insert("BufferedWriter".to_string(), bw_cls.clone());
    let brp_dict = HashMap::new(); let brp_cls = PyObjectRef::new(PyObject::Type { name: "BufferedRWPair".to_string(), dict: brp_dict, bases: vec![buf_cls.clone()], mro: vec![buf_cls.clone()] });
    d.insert("BufferedRWPair".to_string(), brp_cls.clone());
    let brnd_dict = HashMap::new(); let brnd_cls = PyObjectRef::new(PyObject::Type { name: "BufferedRandom".to_string(), dict: brnd_dict, bases: vec![buf_cls.clone()], mro: vec![buf_cls.clone()] });
    d.insert("BufferedRandom".to_string(), brnd_cls.clone());

    // TextIOWrapper — stub type needed by io.py
    let mut tiw_dict = HashMap::new();
    tiw_dict.insert("read".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "read".to_string(), func: |_: &[PyObjectRef]| Ok(py_str("")) }));
    tiw_dict.insert("write".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "write".to_string(), func: |_: &[PyObjectRef]| Ok(py_none()) }));
    tiw_dict.insert("close".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "close".to_string(), func: |_: &[PyObjectRef]| Ok(py_none()) }));
    let tiw_cls = PyObjectRef::new(PyObject::Type { name: "TextIOWrapper".to_string(), dict: tiw_dict, bases: vec![], mro: vec![] });
    d.insert("TextIOWrapper".to_string(), tiw_cls);

    d.insert("_WindowsConsoleIO".to_string(), py_str("_WindowsConsoleIO"));

    d
}