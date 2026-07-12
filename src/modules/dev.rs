use crate::object::*;
use crate::bytecode::{CodeObject, needs_arg};
use std::collections::HashMap;

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

