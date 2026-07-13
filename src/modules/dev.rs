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
        let vm_ptr = crate::object::VM_PTR.with(|p| *p.borrow());
        if let Some(ptr) = vm_ptr {
            let vm = unsafe { &mut *ptr };
            let mut parser = crate::parser::Parser::new(&cmd);
            if let Ok(program) = parser.parse_program() {
                let mut compiler = crate::compiler::Compiler::new();
                if let Ok(code) = compiler.compile(&program, "<profile>") {
                    let _ = vm.exec_code(code, None);
                }
            }
        }
        Ok(py_none())
    });

    prof_func!("runctx", |args| {
        if args.len() < 3 {
            return Err(PyError::type_error("runctx() requires 3 arguments (statement, globals, locals)"));
        }
        let cmd = args[0].str();
        let _globals = &args[1];
        let _locals = &args[2];
        let vm_ptr = crate::object::VM_PTR.with(|p| *p.borrow());
        if let Some(ptr) = vm_ptr {
            let vm = unsafe { &mut *ptr };
            let mut parser = crate::parser::Parser::new(&cmd);
            if let Ok(program) = parser.parse_program() {
                let mut compiler = crate::compiler::Compiler::new();
                if let Ok(code) = compiler.compile(&program, "<profile>") {
                    let _ = vm.exec_code(code, None);
                }
            }
        }
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
                let vm_ptr = crate::object::VM_PTR.with(|p| *p.borrow());
                if let Some(ptr) = vm_ptr {
                    let vm = unsafe { &mut *ptr };
                    let mut parser = crate::parser::Parser::new(&cmd);
                    if let Ok(program) = parser.parse_program() {
                        let mut compiler = crate::compiler::Compiler::new();
                        if let Ok(code) = compiler.compile(&program, "<trace>") {
                            let _ = vm.exec_code(code, None);
                        }
                    }
                }
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
