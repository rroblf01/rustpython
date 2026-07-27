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

    warn_func!("resetwarnings", |_| Ok(py_none()));
    warn_func!("filterwarnings", |_| Ok(py_none()));
    // Real CPython's `warnings.py` does `from _warnings import (..., _deprecated,
    // ...)` — a native-extension-only helper `nturl2path`/other stdlib
    // modules call directly (`warnings._deprecated("urllib.request...",
    // remove=(3, 15))`) to emit a standard-shaped `DeprecationWarning`.
    // Added as a thin wrapper around `warn` (this interpreter's own `warn`
    // just prints, so the exact message shape matters less than not
    // raising `AttributeError` on import).
    d.insert("_DEPRECATED_MSG".to_string(), py_str("{name!r} is deprecated"));
    warn_func!("_deprecated", |args| {
        let name = if !args.is_empty() { args[0].str() } else { String::new() };
        println!("DeprecationWarning: {} is deprecated", name);
        Ok(py_none())
    });

    // catch_warnings() — a context manager real code uses to isolate/mute
    // warning state for a block (real trigger: CPython 3.14's own
    // `unittest/runner.py`, `with warnings.catch_warnings(): ...` around
    // each test run). Simplified: `record=True` returns a (permanently
    // empty, since `warn()` here just prints rather than recording)
    // list — good enough for code that only checks "were there 0 warnings"
    // or iterates expecting none, not for code asserting on captured
    // messages.
    let mut cw_dict = HashMap::new();
    cw_dict.insert("__init__".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "__init__".to_string(),
        func: |args| {
            if let PyObject::Instance { dict, .. } = &mut *args[0].borrow_mut() {
                let record = args.iter().skip(1).any(|a| matches!(&*a.borrow(), PyObject::Dict(d) if d.get(&py_str("record")).ok().flatten().map(|v| v.truthy()).unwrap_or(false)));
                dict.insert("_record".to_string(), py_bool(record));
            }
            Ok(py_none())
        },
    }));
    cw_dict.insert("__enter__".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "__enter__".to_string(),
        func: |args| {
            let record = if let PyObject::Instance { dict, .. } = &*args[0].borrow() {
                dict.get("_record").map(|v| v.truthy()).unwrap_or(false)
            } else { false };
            if record { Ok(py_list(vec![])) } else { Ok(py_none()) }
        },
    }));
    cw_dict.insert("__exit__".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "__exit__".to_string(),
        func: |_args| Ok(py_bool(false)),
    }));
    d.insert("catch_warnings".to_string(), PyObjectRef::new(PyObject::Type {
        name: "catch_warnings".to_string(),
        dict: Box::new(cw_dict),
        bases: vec![],
        mro: vec![],
    }));

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
                dict: Box::new(HashMap::new()),
            }),
            dict: AttrMap::new(),
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

    // ABCMeta — minimal metaclass stub (needed by io.py et al)
    abc_func!("ABCMeta", |args| {
        // In CPython, ABCMeta(name, bases, namespace) -> new class
        // For our stub, return a Type object with the given name
        let name = if !args.is_empty() { args[0].str() } else { "ABCMeta".to_string() };
        Ok(PyObjectRef::new(PyObject::Type {
            name,
            dict: Box::new(HashMap::new()),
            bases: vec![],
            mro: vec![],
        }))
    });

    d
}

pub fn create_dataclasses_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! dc_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    // dataclass(cls) — decorator that marks cls with _dataclass_ attr.
    // NOTE: still just a marker, not real dataclass semantics — a genuine
    // @dataclass should synthesize __init__/__repr__/__eq__/__hash__ from
    // the class's own annotated fields (respecting field()'s
    // default/default_factory/kw_only/etc.), which this does not do at all.
    // That's a separately-tracked, larger gap; this function only fixes the
    // narrower, previously-crashing case of the PARAMETERIZED decorator
    // form (`@dataclass(frozen=True)`, real code — e.g. CPython's own
    // `_colorize.py` theme classes): called with kwargs only (no `cls`
    // positional argument, since our calling convention packs kwargs as a
    // trailing dict arg), this used to just return that kwargs dict itself,
    // which the decorator machinery then tried to CALL as if it were the
    // real decorator — "'dict' object is not callable". Detect that shape
    // and return a real closure instead.
    fn is_class_or_instance(v: &PyObjectRef) -> bool {
        matches!(&*v.borrow(), PyObject::Type { .. } | PyObject::Instance { .. })
    }
    fn mark_dataclass(cls: &PyObjectRef) -> PyResult<PyObjectRef> {
        let mut borrowed = cls.borrow_mut();
        if let PyObject::Instance { ref mut dict, .. } = &mut *borrowed {
            dict.insert("_dataclass_".to_string(), py_bool(true));
        }
        if let PyObject::Type { ref mut dict, .. } = &mut *borrowed {
            dict.insert("_dataclass_".to_string(), py_bool(true));
        }
        drop(borrowed);
        Ok(cls.clone())
    }
    dc_func!("dataclass", |args| {
        if args.is_empty() || !is_class_or_instance(&args[0]) {
            // Parameterized form: @dataclass(frozen=True, ...) — return a
            // decorator closure that applies the same (still-stub) marking
            // once the real class is passed to it.
            return Ok(PyObjectRef::imm(PyObject::Closure(std::rc::Rc::new(|inner_args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                if inner_args.is_empty() {
                    return Err(PyError::type_error("dataclass() decorator missing required argument (cls)"));
                }
                mark_dataclass(&inner_args[0])
            }))));
        }
        mark_dataclass(&args[0])
    });

    // field() — returns empty dict as a field descriptor
    dc_func!("field", |_| {
        Ok(py_dict())
    });

    // Field — the class dataclasses.field()/introspection would normally
    // return/expose (real attrs: name, type, default, default_factory,
    // repr, hash, init, compare, metadata, kw_only). This bare marker class
    // exists mainly so `from dataclasses import Field` succeeds at all
    // (needed transitively by real CPython stdlib source we vendor
    // verbatim, e.g. `_colorize.py`'s own type annotations) — actual
    // dataclass field generation is still a stub (see the `dataclass()`
    // function above: real @dataclass should synthesize __init__/__repr__/
    // __eq__ from annotated fields; this only tags the class, a known,
    // separately-tracked gap, not something this placeholder fixes.
    d.insert("Field".to_string(), PyObjectRef::new(PyObject::Type {
        name: "Field".to_string(),
        dict: Box::new(HashMap::new()),
        bases: vec![],
        mro: vec![],
    }));

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
                Ok(PyObjectRef::new(PyObject::Dict(Box::new(new_dict))))
            }
            PyObject::Dict(pydict) => {
                let mut new_dict = PyDict::new();
                for (k, v) in pydict.items() {
                    let _ = new_dict.set(k, v);
                }
                Ok(PyObjectRef::new(PyObject::Dict(Box::new(new_dict))))
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
            dict: Box::new(HashMap::new()),
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
        dict: Box::new(tc_dict),
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
            PyObject::Function(ref f) => Ok((*f.code).clone()),
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
    // Real CPython's `dis` re-exports these opcode-classification lists
    // from `opcode` (which describes CPython's OWN bytecode format — not
    // this interpreter's, so there's nothing meaningful to populate them
    // with). Empty lists here are enough for code that merely imports/
    // constructs a `set()` from them without asserting real CPython opcode
    // membership (real trigger: `test.support.bytecode_helper`, which our
    // fundamentally-different bytecode format can't produce accurate
    // results for regardless).
    for name in ["hasarg", "hasconst", "hasname", "hasjrel", "hasjabs", "haslocal", "hascompare", "hasfree", "hasexc"] {
        d.insert(name.to_string(), py_list(vec![]));
    }

    d
}

/// Minimal `_opcode` (the CPython C extension backing parts of `dis`).
/// Only exposes the two constants `test.support` itself reads at import
/// time (`ENABLE_SPECIALIZATION`/`ENABLE_SPECIALIZATION_FT`, both about
/// CPython 3.11+'s adaptive specializing interpreter — always `False`
/// here, correct since this interpreter has no such optimization to gate).
pub fn create_opcode_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    d.insert("ENABLE_SPECIALIZATION".to_string(), py_bool(false));
    d.insert("ENABLE_SPECIALIZATION_FT".to_string(), py_bool(false));
    // stack_effect(opcode, oparg) -> int: return the stack effect of an opcode
    d.insert("stack_effect".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "stack_effect".to_string(),
        func: |args| {
            if args.is_empty() { return Err(PyError::type_error("stack_effect() missing required argument")); }
            // Return a conservative estimate (2 for most ops, 0 for simple)
            let opcode_str = args[0].str();
            match opcode_str.as_str() {
                "RETURN_VALUE" | "POP_TOP" => Ok(py_int(-1)),
                "LOAD_CONST" | "LOAD_FAST" | "LOAD_NAME" | "LOAD_GLOBAL" | "LOAD_DEREF" => Ok(py_int(1)),
                "BUILD_LIST" | "BUILD_TUPLE" | "BUILD_SET" | "BUILD_MAP" | "BUILD_STRING" => Ok(py_int(1 - args.get(1).and_then(|a| a.as_i64()).unwrap_or(1) as i64)),
                "CALL" | "CALL_FUNCTION_EX" | "CALL_KW" => Ok(py_int(-1)),
                _ => Ok(py_int(0)),
            }
        },
    }));
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
        let mut dict = AttrMap::new();
        dict.insert("failed".to_string(), py_int(0));
        dict.insert("attempted".to_string(), py_int(0));
        Ok(PyObjectRef::new(PyObject::Instance {
            typ: py_str("TestResults"),
            dict,
        }))
    });

    // testmod(m=None) — runs doctests on a module, returns TestResults(failed=0, attempted=0)
    doctest_func!("testmod", |_args| {
        let mut dict = AttrMap::new();
        dict.insert("failed".to_string(), py_int(0));
        dict.insert("attempted".to_string(), py_int(0));
        Ok(PyObjectRef::new(PyObject::Instance {
            typ: py_str("TestResults"),
            dict,
        }))
    });

    // testfile(filename) — runs doctests in a file, returns TestResults(failed=0, attempted=0)
    doctest_func!("testfile", |_args| {
        let mut dict = AttrMap::new();
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
        let mut dict = AttrMap::new();
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

    // A unique "no value given" marker distinct from `None` (real code uses
    // it as a default-argument sentinel so `None` remains a legitimate
    // explicit value) — real trigger: CPython's own `test.support`,
    // `find_name_in_mro(cls, name, default=inspect._sentinel)`. Any
    // distinct object identity works; a bare Instance of an empty marker
    // Type is the simplest one available.
    d.insert("_sentinel".to_string(), PyObjectRef::new(PyObject::Instance {
        typ: PyObjectRef::new(PyObject::Type {
            name: "_sentinel".to_string(),
            dict: Box::new(HashMap::new()),
            bases: vec![],
            mro: vec![],
        }),
        dict: AttrMap::new(),
    }));

    inspect_func!("isfunction", |args| {
        if args.len() < 1 { return Err(PyError::type_error("isfunction() requires 1 argument")); }
        let obj = args[0].borrow();
        Ok(py_bool(matches!(&*obj, PyObject::Function(_))))
    });

    inspect_func!("isgeneratorfunction", |args| {
        if args.len() < 1 { return Err(PyError::type_error("isgeneratorfunction() requires 1 argument")); }
        let obj = args[0].borrow();
        let is_gen = match &*obj {
            PyObject::Function(ref f) => (f.code.flags & 0x0020) != 0,
            _ => false,
        };
        Ok(py_bool(is_gen))
    });

    inspect_func!("iscoroutinefunction", |args| {
        if args.len() < 1 { return Err(PyError::type_error("iscoroutinefunction() requires 1 argument")); }
        let obj = args[0].borrow();
        let is_coro = match &*obj {
            PyObject::Function(ref f) => (f.code.flags & 0x0080) != 0,
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

    // isabstract(cls) — real CPython checks `bool(getattr(cls,
    // '__abstractmethods__', False))`, populated by ABCMeta. This
    // interpreter's `abc.ABC`/`ABCMeta` are still a stub that never
    // populates `__abstractmethods__` at all, so nothing can ever actually
    // be an abstract class here yet — always False is correct for now,
    // matching what a class with no abstract methods should report.
    inspect_func!("isabstract", |args| {
        if args.len() < 1 { return Err(PyError::type_error("isabstract() requires 1 argument")); }
        let obj = args[0].borrow();
        let has_abstract_methods = match &*obj {
            PyObject::Type { dict, .. } => dict.get("__abstractmethods__")
                .map(|v| v.truthy())
                .unwrap_or(false),
            _ => false,
        };
        Ok(py_bool(has_abstract_methods))
    });

    inspect_func!("getdoc", |args| {
        if args.len() < 1 { return Err(PyError::type_error("getdoc() requires 1 argument")); }
        let obj = args[0].borrow();
        let doc = match &*obj {
            PyObject::Function(ref f) => f.dict.get("__doc__").cloned(),
            PyObject::Type { ref dict, .. } => dict.get("__doc__").cloned(),
            PyObject::Module { ref dict, .. } => dict.get("__doc__").cloned(),
            PyObject::Instance { ref dict, .. } => dict.get("__doc__").cloned(),
            _ => None,
        };
        Ok(doc.unwrap_or(py_none()))
    });

    inspect_func!("getfile", |args| {
        if args.is_empty() { return Err(PyError::type_error("getfile() requires 1 argument")); }
        let obj = args[0].borrow();
        // Try to get __code__ attribute
        if let Ok(code) = obj.get_attribute("__code__") {
            let code_borrowed = code.borrow();
            if let PyObject::Code(c) = &*code_borrowed {
                return Ok(py_str(&c.filename));
            }
        }
        Ok(py_str("<unknown>"))
    });
    inspect_func!("getsourcefile", |args| {
        if args.is_empty() { return Err(PyError::type_error("getsourcefile() requires 1 argument")); }
        let obj = args[0].borrow();
        if let Ok(code) = obj.get_attribute("__code__") {
            let code_borrowed = code.borrow();
            if let PyObject::Code(c) = &*code_borrowed {
                return Ok(py_str(&c.filename));
            }
        }
        Ok(py_none())
    });
    inspect_func!("getsource", |args| {
        if args.is_empty() { return Err(PyError::type_error("getsource() requires 1 argument")); }
        let obj = args[0].borrow();
        let filename = obj.get_attribute("__code__").ok().and_then(|code| {
            let code_borrowed = code.borrow();
            if let PyObject::Code(c) = &*code_borrowed { Some(c.filename.clone()) } else { None }
        });
        if let Some(fname) = filename {
            if let Ok(src) = std::fs::read_to_string(&fname) {
                return Ok(py_str(&src));
            }
        }
        Ok(py_str("Source not available in RustPython"))
    });

    inspect_func!("getmodule", |args| {
        if args.len() < 1 { return Err(PyError::type_error("getmodule() requires 1 argument")); }
        let module_name = args[0].borrow().get_attribute("__module__").ok()
            .and_then(|v| { if let PyObject::Str(s) = &*v.borrow() { Some(s.to_string()) } else { None } });
        Ok(if let Some(name) = module_name { py_str(&name) } else { py_none() })
    });

    inspect_func!("getmembers", getmembers_builtin);

    inspect_func!("getfullargspec", |args| {
        if args.is_empty() { return Err(PyError::type_error("getfullargspec() requires 1 argument")); }
        let target = match &*args[0].borrow() {
            PyObject::BoundMethod { func, .. } => func.clone(),
            _ => args[0].clone(),
        };
        let b = target.borrow();
        if let PyObject::Function(ref inner_f) = &*b {
        let code = &inner_f.code;
        let defaults = &inner_f.defaults;
            let arg_count = code.arg_count.min(code.varnames.len());
            let positional_args: Vec<PyObjectRef> = code.varnames[..arg_count].iter().map(|n| py_str(n)).collect();
            // varnames layout is: positional args, then *args (if any), then
            // kwonly args, then **kwargs (if any) — the vararg slot must be
            // skipped when locating where kwonly names start.
            let kwonly_start = arg_count + if code.vararg_name.is_some() { 1 } else { 0 };
            let kwonlyargs: Vec<PyObjectRef> = if code.kwonlyarg_count > 0 {
                code.varnames.get(kwonly_start..kwonly_start + code.kwonlyarg_count)
                    .map(|s| s.iter().map(|n| py_str(n)).collect())
                    .unwrap_or_default()
            } else {
                vec![]
            };
            let varargs = code.vararg_name.as_ref().map(|n| py_str(n)).unwrap_or_else(py_none);
            let varkw = code.kwarg_name.as_ref().map(|n| py_str(n)).unwrap_or_else(py_none);
            // `defaults` holds positional defaults then kwonly ones appended
            // after (see MAKE_FUNCTION/CodeObject::kwonly_defaults_mask) —
            // code.num_defaults is the positional-only count.
            let num_defaults = code.num_defaults;
            let defaults_val = if num_defaults == 0 { py_none() } else { py_tuple(defaults[..num_defaults].to_vec()) };
            let kwonlydefaults = py_dict();
            if code.kwonlyarg_count > 0 {
                let mut kwdefault_idx = num_defaults;
                if let PyObject::Dict(d) = &mut *kwonlydefaults.borrow_mut() {
                    for (k, has_default) in code.kwonly_defaults_mask.iter().enumerate() {
                        if !*has_default { continue; }
                        if let Some(pname) = code.varnames.get(kwonly_start + k) {
                            if let Some(v) = defaults.get(kwdefault_idx) {
                                d.set(py_str(pname), v.clone())?;
                            }
                        }
                        kwdefault_idx += 1;
                    }
                }
            }
            let kwonlydefaults = if kwonlyargs.is_empty() || matches!(&*kwonlydefaults.borrow(), PyObject::Dict(d) if d.is_empty()) {
                py_none()
            } else {
                kwonlydefaults
            };
            Ok(py_tuple(vec![
                py_list(positional_args),
                varargs,
                varkw,
                defaults_val,
                py_list(kwonlyargs),
                kwonlydefaults,
                py_dict(),
            ]))
        } else {
            Err(PyError::type_error("getfullargspec() requires a Python function"))
        }
    });

    inspect_func!("unwrap", |args| {
        if args.is_empty() { return Err(PyError::type_error("unwrap() requires 1 argument")); }
        let mut current = args[0].clone();
        let mut seen = std::collections::HashSet::new();
        loop {
            let next = current.borrow().get_attribute("__wrapped__").ok();
            match next {
                Some(w) => {
                    if !seen.insert(w.get_id()) { break; }
                    current = w;
                }
                None => break,
            }
        }
        Ok(current)
    });

    inspect_func!("signature", |args| {
        if args.is_empty() { return Err(PyError::type_error("signature() requires 1 argument")); }
        let target = match &*args[0].borrow() {
            PyObject::BoundMethod { func, .. } => func.clone(),
            _ => args[0].clone(),
        };
        let b = target.borrow();
        if let PyObject::Function(ref inner_f) = &*b {
        let code = &inner_f.code;
        let defaults = &inner_f.defaults;
            let mut param_type_dict = HashMap::new();
            param_type_dict.insert("POSITIONAL_ONLY".to_string(), py_int(0));
            param_type_dict.insert("POSITIONAL_OR_KEYWORD".to_string(), py_int(1));
            param_type_dict.insert("VAR_POSITIONAL".to_string(), py_int(2));
            param_type_dict.insert("KEYWORD_ONLY".to_string(), py_int(3));
            param_type_dict.insert("VAR_KEYWORD".to_string(), py_int(4));
            param_type_dict.insert("empty".to_string(), py_none());
            let param_type = PyObjectRef::new(PyObject::Type { name: "Parameter".to_string(), dict: Box::new(param_type_dict), bases: vec![], mro: vec![] });
            let make_param = |pname: &str, kind: i64, default: PyObjectRef, param_type: &PyObjectRef| {
                let mut inst_dict = AttrMap::new();
                inst_dict.insert("name".to_string(), py_str(pname));
                inst_dict.insert("kind".to_string(), py_int(kind));
                inst_dict.insert("default".to_string(), default);
                PyObjectRef::new(PyObject::Instance { typ: param_type.clone(), dict: inst_dict })
            };
            let mut params = PyDict::new();
            let arg_count = code.arg_count.min(code.varnames.len());
            // `defaults` holds positional defaults THEN keyword-only ones
            // appended after (see MAKE_FUNCTION/CodeObject::kwonly_defaults_mask)
            // — code.num_defaults is the count of just the positional ones;
            // defaults.len() also counts the kwonly tail, which would shift
            // every positional default computed from it by however many
            // kwonly defaults exist.
            let num_defaults = code.num_defaults;
            let first_default_idx = arg_count.saturating_sub(num_defaults);
            for i in 0..arg_count {
                let pname = code.varnames[i].clone();
                let default = if i >= first_default_idx { defaults[i - first_default_idx].clone() } else { py_none() };
                let p = make_param(&pname, 1, default, &param_type); // POSITIONAL_OR_KEYWORD
                params.set(py_str(&pname), p)?;
            }
            if let Some(va) = &code.vararg_name {
                let p = make_param(va, 2, py_none(), &param_type); // VAR_POSITIONAL
                params.set(py_str(va), p)?;
            }
            // varnames layout is: positional args, then *args (if any), then
            // kwonly args, then **kwargs (if any) — the vararg slot must be
            // skipped when locating where kwonly names start.
            let kwonly_start = arg_count + if code.vararg_name.is_some() { 1 } else { 0 };
            if code.kwonlyarg_count > 0 {
                let mut kwdefault_idx = num_defaults;
                if let Some(kwonly) = code.varnames.get(kwonly_start..kwonly_start + code.kwonlyarg_count) {
                    for (k, pname) in kwonly.iter().enumerate() {
                        let has_default = code.kwonly_defaults_mask.get(k).copied().unwrap_or(false);
                        let default = if has_default {
                            let v = defaults.get(kwdefault_idx).cloned().unwrap_or_else(py_none);
                            kwdefault_idx += 1;
                            v
                        } else {
                            py_none()
                        };
                        let p = make_param(pname, 3, default, &param_type); // KEYWORD_ONLY
                        params.set(py_str(pname), p)?;
                    }
                }
            }
            if let Some(kw) = &code.kwarg_name {
                let p = make_param(kw, 4, py_none(), &param_type); // VAR_KEYWORD
                params.set(py_str(kw), p)?;
            }
            let sig_type = PyObjectRef::new(PyObject::Type { name: "Signature".to_string(), dict: Box::new(HashMap::new()), bases: vec![], mro: vec![] });
            let mut sig_dict = AttrMap::new();
            sig_dict.insert("parameters".to_string(), PyObjectRef::new(PyObject::Dict(Box::new(params))));
            Ok(PyObjectRef::new(PyObject::Instance { typ: sig_type, dict: sig_dict }))
        } else {
            // Real CPython raises ValueError here (not TypeError) — "no
            // signature found for builtin ..." — since a builtin/native
            // callable genuinely has no introspectable signature, as
            // opposed to the argument not being callable at all. Matters
            // beyond cosmetics: `unittest/mock.py`'s own module-level
            // `inspect.signature(partial(CodeType.__init__, None))` is
            // wrapped in `except ValueError:` specifically expecting this.
            Err(PyError::value_error("no signature found for builtin type"))
        }
    });
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
    d.insert("Parameter".to_string(), PyObjectRef::new(PyObject::Type { name: "Parameter".to_string(), dict: Box::new(param_type_dict), bases: vec![], mro: vec![] }));
    d.insert("Signature".to_string(), PyObjectRef::new(PyObject::Type { name: "Signature".to_string(), dict: Box::new(HashMap::new()), bases: vec![], mro: vec![] }));

    // Code object flags (CO_* constants)
    d.insert("CO_OPTIMIZED".to_string(), py_int(0x0001));
    d.insert("CO_NEWLOCALS".to_string(), py_int(0x0002));
    d.insert("CO_VARARGS".to_string(), py_int(0x0004));
    d.insert("CO_VARKEYWORDS".to_string(), py_int(0x0008));
    d.insert("CO_NESTED".to_string(), py_int(0x0010));
    d.insert("CO_GENERATOR".to_string(), py_int(0x0020));
    d.insert("CO_NOFREE".to_string(), py_int(0x0040));
    d.insert("CO_COROUTINE".to_string(), py_int(0x0080));
    d.insert("CO_ITERABLE_COROUTINE".to_string(), py_int(0x0100));
    d.insert("CO_ASYNC_GENERATOR".to_string(), py_int(0x0200));
    d.insert("CO_FUTURE_DIVISION".to_string(), py_int(0x2000));
    d.insert("CO_FUTURE_ABSOLUTE_IMPORT".to_string(), py_int(0x4000));
    d.insert("CO_FUTURE_WITH_STATEMENT".to_string(), py_int(0x8000));
    d.insert("CO_FUTURE_PRINT_FUNCTION".to_string(), py_int(0x10000));
    d.insert("CO_FUTURE_UNICODE_LITERALS".to_string(), py_int(0x20000));
    d.insert("CO_FUTURE_BARRY_AS_BDFL".to_string(), py_int(0x40000));
    d.insert("CO_FUTURE_GENERATOR_STOP".to_string(), py_int(0x80000));
    d.insert("CO_FUTURE_ANNOTATIONS".to_string(), py_int(0x100000));

    d
}

fn getmembers_dict_of(obj: &PyObjectRef) -> Vec<(String, PyObjectRef)> {
    let b = obj.borrow();
    let mut items: Vec<(String, PyObjectRef)> = match &*b {
        PyObject::Function(ref f) => f.dict.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        PyObject::Type { dict, .. } => dict.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        PyObject::Module { dict, .. } => dict.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        PyObject::Instance { dict, .. } => dict.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        _ => Vec::new(),
    };
    items.sort_by(|a, b| a.0.cmp(&b.0));
    items
}

/// `inspect.getmembers(object, predicate=None)`, given genuine `&mut
/// VirtualMachine` access to actually call `predicate` on each candidate —
/// called directly from `vm.rs`'s `call_function` special-case (see
/// `is_getmembers`) for the same reason `find_spec`/`getattr`/`import_module`
/// are special-cased there: this is reached from deep inside real Django
/// app-loading code (`inspect.getmembers(mod, inspect.isclass)`), where
/// `with_vm_mut`'s reentrancy hazard applies.
pub(crate) fn getmembers_with_vm(vm: &mut crate::vm::VirtualMachine, obj: &PyObjectRef, predicate: Option<&PyObjectRef>) -> PyResult<PyObjectRef> {
    let items = getmembers_dict_of(obj);
    let mut members = Vec::new();
    for (k, v) in items {
        let keep = match predicate {
            Some(p) => vm.call_function(p.clone(), vec![v.clone()], vec![])?.truthy(),
            None => true,
        };
        if keep {
            members.push(py_tuple(vec![py_str(&k), v]));
        }
    }
    Ok(py_list(members))
}

/// `getmembers`'s standalone entry point (predicate not called through the
/// real VM) — used only if reached outside `vm.rs`'s special-cased dispatch.
/// Note: this fallback can't safely invoke a Python-level predicate (that's
/// exactly the reentrancy hazard `getmembers_with_vm` exists to avoid), so it
/// silently ignores `predicate` and returns everything, matching this
/// function's pre-existing (if incomplete) behavior for that fallback path.
pub(crate) fn getmembers_builtin(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() { return Err(PyError::type_error("getmembers() requires 1 argument")); }
    let items = getmembers_dict_of(&args[0]);
    Ok(py_list(items.into_iter().map(|(k, v)| py_tuple(vec![py_str(&k), v])).collect()))
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
        let mut inst_dict = AttrMap::new();
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
        let mut result_dict = AttrMap::new();
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
        let mut inst_dict = AttrMap::new();
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
        let mut inst_dict = AttrMap::new();
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
    imp_func!("exec_builtin", |_args| {
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
    // Both were bare `py_none()` placeholders — not callable at all — which
    // broke `test.support.import_helper.frozen_modules()`/
    // `multi_interp_extensions_check()` (both real CPython context managers
    // wrapping a call to one of these) with `TypeError: 'NoneType' object is
    // not callable`, for any test file using `import_fresh_module`/`CleanImport`
    // (an extremely common test-infrastructure idiom — real trigger: 10+
    // corpus files hit this identical symptom via `import_helper`).
    imp_func!("_override_frozen_modules_for_tests", |_| Ok(py_none()));
    imp_func!("_override_multi_interp_extensions_check", |_| Ok(py_none()));

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
        let _path = if !args.is_empty() { args[0].str() } else { String::new() };
        let mut inst_dict = AttrMap::new();
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
        Ok(PyObjectRef::new(PyObject::File { file: Rc::new(RefCell::new(file)), name: filename.clone() }))
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

        type_dict.insert("__init__".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__init__".to_string(), func: |_: &[PyObjectRef]| Ok(py_none()),
        }));

        let b_read = buf_rc.clone();
        let p_read = pos_rc.clone();
        type_dict.insert("read".to_string(), PyObjectRef::new(PyObject::Closure(Rc::new(move |args: &[PyObjectRef]| {
            let data = b_read.borrow();
            let pos = (*p_read.borrow()).min(data.len());
            let end = if !args.is_empty() {
                args[0].as_i64().filter(|&n| n >= 0).map(|n| (pos + n as usize).min(data.len())).unwrap_or(data.len())
            } else {
                data.len()
            };
            let chunk = data[pos..end].to_vec();
            *p_read.borrow_mut() = end;
            Ok(PyObjectRef::imm(PyObject::Bytes(chunk)))
        }))));

        let b_readline = buf_rc.clone();
        let p_readline = pos_rc.clone();
        type_dict.insert("readline".to_string(), PyObjectRef::new(PyObject::Closure(Rc::new(move |_: &[PyObjectRef]| {
            let data = b_readline.borrow();
            let pos = (*p_readline.borrow()).min(data.len());
            let remaining = &data[pos..];
            let end = remaining.iter().position(|&c| c == b'\n').map(|i| i + 1).unwrap_or(remaining.len());
            let chunk = remaining[..end].to_vec();
            *p_readline.borrow_mut() = pos + end;
            Ok(PyObjectRef::imm(PyObject::Bytes(chunk)))
        }))));

        let b_write = buf_rc.clone();
        let p_write = pos_rc.clone();
        type_dict.insert("write".to_string(), PyObjectRef::new(PyObject::Closure(Rc::new(move |w_args: &[PyObjectRef]| {
            if w_args.is_empty() {
                return Err(PyError::type_error("write() takes exactly one argument"));
            }
            let data = match &*w_args[0].borrow() {
                PyObject::Bytes(b) => b.clone(),
                PyObject::ByteArray(b) => b.clone(),
                _ => return Err(PyError::type_error("a bytes-like object is required, not str")),
            };
            let mut buf = b_write.borrow_mut();
            let pos = *p_write.borrow();
            if pos + data.len() > buf.len() {
                buf.resize(pos, 0);
                buf.extend_from_slice(&data);
            } else {
                buf[pos..pos + data.len()].copy_from_slice(&data);
            }
            *p_write.borrow_mut() = pos + data.len();
            Ok(py_int(data.len() as i64))
        }))));

        let b_seek = buf_rc.clone();
        let p_seek = pos_rc.clone();
        type_dict.insert("seek".to_string(), PyObjectRef::new(PyObject::Closure(Rc::new(move |s_args: &[PyObjectRef]| {
            let offset = s_args.first().and_then(|a| a.as_i64()).unwrap_or(0);
            let whence = s_args.get(1).and_then(|a| a.as_i64()).unwrap_or(0);
            let len = b_seek.borrow().len() as i64;
            let base = match whence { 1 => *p_seek.borrow() as i64, 2 => len, _ => 0 };
            let new_pos = (base + offset).max(0) as usize;
            *p_seek.borrow_mut() = new_pos;
            Ok(py_int(new_pos as i64))
        }))));

        let p_tell = pos_rc.clone();
        type_dict.insert("tell".to_string(), PyObjectRef::new(PyObject::Closure(Rc::new(move |_: &[PyObjectRef]| {
            Ok(py_int(*p_tell.borrow() as i64))
        }))));

        let b_getvalue = buf_rc.clone();
        type_dict.insert("getvalue".to_string(), PyObjectRef::new(PyObject::Closure(Rc::new(move |_: &[PyObjectRef]| {
            Ok(PyObjectRef::imm(PyObject::Bytes(b_getvalue.borrow().clone())))
        }))));

        type_dict.insert("close".to_string(), PyObjectRef::new(PyObject::Closure(Rc::new(move |_: &[PyObjectRef]| Ok(py_none())))));

        Ok(PyObjectRef::new(PyObject::Instance {
            typ: PyObjectRef::new(PyObject::Type { name: "BytesIO".to_string(), dict: Box::new(type_dict), bases: vec![], mro: vec![] }),
            dict: AttrMap::new(),
        }))
    });

    // IncrementalNewlineDecoder — stub
    io_func!("IncrementalNewlineDecoder", |_args| {
        let mut type_dict = AttrMap::new();
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
        Ok(PyObjectRef::new(PyObject::File { file: Rc::new(RefCell::new(file)), name: path.clone() }))
    });

    io_func!("text_encoding", |args| {
        if args.is_empty() { return Err(PyError::type_error("text_encoding() missing argument")); }
        Ok(py_str(&args[0].str()))
    });

    d.insert("open".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "open".to_string(), func: builtin_open,
    }));
    d.insert("DEFAULT_BUFFER_SIZE".to_string(), py_int(8192));

    // BlockingIOError — exception type (needs to support attribute setting like __module__)
    d.insert("BlockingIOError".to_string(), PyObjectRef::new(PyObject::Type {
        name: "BlockingIOError".to_string(),
        dict: Box::new(HashMap::new()),
        bases: vec![],
        mro: vec![],
    }));

    // UnsupportedOperation — exception type (needs __module__ set by io.py)
    let mut uo_dict = HashMap::new();
    uo_dict.insert("__init__".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "__init__".to_string(), func: |_: &[PyObjectRef]| Ok(py_none()),
    }));
    d.insert("UnsupportedOperation".to_string(), PyObjectRef::new(PyObject::Type {
        name: "UnsupportedOperation".to_string(),
        dict: Box::new(uo_dict),
        bases: vec![],
        mro: vec![],
    }));

    // ── IO Base Classes ─────────────────────────────────────────────────────────

    // IOBase — abstract base class with close, closed, __enter__, __exit__
    let mut iobase_dict = HashMap::new();
    iobase_dict.insert("__doc__".to_string(), py_str("IOBase abstract class"));
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
        name: "IOBase".to_string(), dict: Box::new(iobase_dict), bases: vec![], mro: vec![],
    });
    d.insert("IOBase".to_string(), iobase_cls.clone());
    d.insert("_IOBase".to_string(), iobase_cls.clone());

    // RawIOBase — extends IOBase
    let mut raw_dict = HashMap::new();
    raw_dict.insert("__doc__".to_string(), py_str("RawIOBase abstract class"));
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
    raw_dict.insert("register".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "register".to_string(), func: |_: &[PyObjectRef]| Ok(py_none()),
    }));
    let raw_cls = PyObjectRef::new(PyObject::Type {
        name: "RawIOBase".to_string(), dict: Box::new(raw_dict),
        bases: vec![iobase_cls.clone()], mro: vec![iobase_cls.clone()],
    });
    d.insert("RawIOBase".to_string(), raw_cls.clone());
    d.insert("_RawIOBase".to_string(), raw_cls.clone());

    // BufferedIOBase — extends IOBase
    let mut buf_dict = HashMap::new();
    buf_dict.insert("__doc__".to_string(), py_str("BufferedIOBase abstract class"));
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
    buf_dict.insert("register".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "register".to_string(), func: |_: &[PyObjectRef]| Ok(py_none()),
    }));
    let buf_cls = PyObjectRef::new(PyObject::Type {
        name: "BufferedIOBase".to_string(), dict: Box::new(buf_dict),
        bases: vec![iobase_cls.clone()], mro: vec![iobase_cls.clone()],
    });
    d.insert("BufferedIOBase".to_string(), buf_cls.clone());
    d.insert("_BufferedIOBase".to_string(), buf_cls.clone());

    // TextIOBase — text I/O base class (extends IOBase)
    let mut text_dict = HashMap::new();
    text_dict.insert("__doc__".to_string(), py_str("TextIOBase abstract class"));
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
    text_dict.insert("register".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "register".to_string(), func: |_: &[PyObjectRef]| Ok(py_none()),
    }));
    let text_cls = PyObjectRef::new(PyObject::Type {
        name: "TextIOBase".to_string(), dict: Box::new(text_dict),
        bases: vec![iobase_cls.clone()], mro: vec![iobase_cls.clone()],
    });
    d.insert("TextIOBase".to_string(), text_cls.clone());
    d.insert("_TextIOBase".to_string(), text_cls.clone());

    // StringIO — real in-memory text buffer with actual position tracking
    // (char-indexed, matching Python's own str model — NOT byte-indexed).
    // The PREVIOUS implementation was a near-total stub: `read()` ignored
    // any size argument and always returned the ENTIRE buffer regardless of
    // position, and `seek`/`tell` were hardcoded to always return 0 — no
    // position tracking existed at all. This made the extremely common
    // `while True: chunk = f.read(1)\n if not chunk: break` idiom loop
    // FOREVER (`read(1)` never shrinks, never returns `''`) — confirmed via
    // CPython's own `shlex.py` (`shlex.split(...)` hung indefinitely on any
    // input). Position is tracked in a `Rc<RefCell<usize>>` (char offset,
    // not byte offset) alongside the buffer.
    let stringio_closure: Rc<dyn Fn(&[PyObjectRef]) -> PyResult<PyObjectRef>> = Rc::new(move |args: &[PyObjectRef]| {
        let initial_value = if !args.is_empty() && !matches!(&*args[0].borrow(), PyObject::None) { args[0].str() } else { String::new() };
        let buffer = Rc::new(RefCell::new(initial_value));
        let pos = Rc::new(RefCell::new(0usize));
        let mut type_dict = HashMap::new();

        // __init__ — no-op (initial_value already consumed by factory)
        type_dict.insert("__init__".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__init__".to_string(), func: |_: &[PyObjectRef]| Ok(py_none()),
        }));

        // Optional size arg: absent, explicit None, or negative all mean
        // "no limit" (read to end / no truncation), matching real
        // `read(size=-1)`/`truncate(size=None)` semantics.
        fn opt_size(args: &[PyObjectRef], idx: usize) -> Option<i64> {
            let a = args.get(idx)?;
            if matches!(&*a.borrow(), PyObject::None) { return None; }
            let n = a.as_i64()?;
            if n < 0 { None } else { Some(n) }
        }

        // read(size=-1) — from the current position, advancing it.
        let (b, p) = (buffer.clone(), pos.clone());
        type_dict.insert("read".to_string(), PyObjectRef::new(PyObject::Closure(Rc::new(move |args: &[PyObjectRef]| {
            let chars: Vec<char> = b.borrow().chars().collect();
            let start = (*p.borrow()).min(chars.len());
            let end = match opt_size(args, 0) {
                Some(n) => (start + n as usize).min(chars.len()),
                None => chars.len(),
            };
            *p.borrow_mut() = end;
            Ok(py_str(&chars[start..end].iter().collect::<String>()))
        }))));

        // readline(size=-1) — up to and including the next '\n', or EOF.
        let (b, p) = (buffer.clone(), pos.clone());
        type_dict.insert("readline".to_string(), PyObjectRef::new(PyObject::Closure(Rc::new(move |args: &[PyObjectRef]| {
            let chars: Vec<char> = b.borrow().chars().collect();
            let start = (*p.borrow()).min(chars.len());
            let limit = opt_size(args, 0).map(|n| (start + n as usize).min(chars.len())).unwrap_or(chars.len());
            let mut end = start;
            while end < limit {
                if chars[end] == '\n' { end += 1; break; }
                end += 1;
            }
            *p.borrow_mut() = end;
            Ok(py_str(&chars[start..end].iter().collect::<String>()))
        }))));

        // write(s) — overwrite at the current position (extending the
        // buffer if writing past its current end), then advance position
        // by the written length. Matches real `StringIO.write`'s
        // "positioned write", not a plain append.
        let (b, p) = (buffer.clone(), pos.clone());
        type_dict.insert("write".to_string(), PyObjectRef::new(PyObject::Closure(Rc::new(move |w_args: &[PyObjectRef]| {
            let text = if !w_args.is_empty() { w_args[0].str() } else { String::new() };
            let mut chars: Vec<char> = b.borrow().chars().collect();
            let start = *p.borrow();
            while chars.len() < start { chars.push('\0'); }
            let new_chars: Vec<char> = text.chars().collect();
            let end = start + new_chars.len();
            if end > chars.len() {
                chars.truncate(start);
                chars.extend(new_chars.iter());
            } else {
                chars.splice(start..end, new_chars.iter().cloned());
            }
            *b.borrow_mut() = chars.into_iter().collect();
            *p.borrow_mut() = end;
            Ok(py_int(text.chars().count() as i64))
        }))));

        // getvalue — full buffer contents regardless of current position.
        let b_get = buffer.clone();
        type_dict.insert("getvalue".to_string(), PyObjectRef::new(PyObject::Closure(Rc::new(move |_: &[PyObjectRef]| {
            Ok(py_str(&b_get.borrow()))
        }))));

        // close — no-op
        type_dict.insert("close".to_string(), PyObjectRef::new(PyObject::Closure(Rc::new(move |_: &[PyObjectRef]| {
            Ok(py_none())
        }))));

        // seek(pos, whence=0) — 0=absolute, 1=relative, 2=from end.
        let (b, p) = (buffer.clone(), pos.clone());
        type_dict.insert("seek".to_string(), PyObjectRef::new(PyObject::Closure(Rc::new(move |args: &[PyObjectRef]| {
            let target = args.get(0).and_then(|a| a.as_i64()).unwrap_or(0);
            let whence = args.get(1).and_then(|a| a.as_i64()).unwrap_or(0);
            let len = b.borrow().chars().count() as i64;
            let new_pos = match whence {
                1 => *p.borrow() as i64 + target,
                2 => len + target,
                _ => target,
            };
            let new_pos = new_pos.max(0) as usize;
            *p.borrow_mut() = new_pos;
            Ok(py_int(new_pos as i64))
        }))));

        // tell — current position.
        let p_tell = pos.clone();
        type_dict.insert("tell".to_string(), PyObjectRef::new(PyObject::Closure(Rc::new(move |_: &[PyObjectRef]| {
            Ok(py_int(*p_tell.borrow() as i64))
        }))));

        // truncate(size=None) — cut the buffer at `size` chars (current
        // position if omitted); does NOT move the current position (matches
        // real `io.StringIO.truncate`).
        let (b, p) = (buffer.clone(), pos.clone());
        type_dict.insert("truncate".to_string(), PyObjectRef::new(PyObject::Closure(Rc::new(move |args: &[PyObjectRef]| {
            let mut chars: Vec<char> = b.borrow().chars().collect();
            let size = opt_size(args, 0).map(|n| n as usize).unwrap_or(*p.borrow()).min(chars.len());
            chars.truncate(size);
            *b.borrow_mut() = chars.into_iter().collect();
            Ok(py_int(size as i64))
        }))));

        // readlines(hint=-1) — split remaining content into lines (each
        // keeping its trailing '\n' except possibly the last).
        let (b, p) = (buffer.clone(), pos.clone());
        type_dict.insert("readlines".to_string(), PyObjectRef::new(PyObject::Closure(Rc::new(move |_: &[PyObjectRef]| {
            let chars: Vec<char> = b.borrow().chars().collect();
            let start = (*p.borrow()).min(chars.len());
            let rest: String = chars[start..].iter().collect();
            *p.borrow_mut() = chars.len();
            let mut lines = Vec::new();
            let mut cur = String::new();
            for c in rest.chars() {
                cur.push(c);
                if c == '\n' { lines.push(py_str(&cur)); cur.clear(); }
            }
            if !cur.is_empty() { lines.push(py_str(&cur)); }
            Ok(py_list(lines))
        }))));

        // __iter__/__next__ — iterate remaining lines, StopIteration at EOF.
        type_dict.insert("__iter__".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__iter__".to_string(), func: |args: &[PyObjectRef]| Ok(args[0].clone()),
        }));
        let (b, p) = (buffer.clone(), pos.clone());
        type_dict.insert("__next__".to_string(), PyObjectRef::new(PyObject::Closure(Rc::new(move |_: &[PyObjectRef]| {
            let chars: Vec<char> = b.borrow().chars().collect();
            let start = (*p.borrow()).min(chars.len());
            if start >= chars.len() { return Err(PyError::StopIteration); }
            let mut end = start;
            while end < chars.len() {
                if chars[end] == '\n' { end += 1; break; }
                end += 1;
            }
            *p.borrow_mut() = end;
            Ok(py_str(&chars[start..end].iter().collect::<String>()))
        }))));

        Ok(PyObjectRef::new(PyObject::Instance {
            typ: PyObjectRef::new(PyObject::Type {
                name: "StringIO".to_string(), dict: Box::new(type_dict),
                bases: vec![text_cls.clone()], mro: vec![text_cls.clone()],
            }),
            dict: AttrMap::new(),
        }))
    });
    d.insert("StringIO".to_string(), PyObjectRef::new(PyObject::Closure(stringio_closure)));

    // BufferedReader, BufferedWriter, BufferedRWPair, BufferedRandom — stubs
    let br_dict = HashMap::new(); let br_cls = PyObjectRef::new(PyObject::Type { name: "BufferedReader".to_string(), dict: Box::new(br_dict), bases: vec![buf_cls.clone()], mro: vec![buf_cls.clone()] });
    d.insert("BufferedReader".to_string(), br_cls.clone());
    let bw_dict = HashMap::new(); let bw_cls = PyObjectRef::new(PyObject::Type { name: "BufferedWriter".to_string(), dict: Box::new(bw_dict), bases: vec![buf_cls.clone()], mro: vec![buf_cls.clone()] });
    d.insert("BufferedWriter".to_string(), bw_cls.clone());
    let brp_dict = HashMap::new(); let brp_cls = PyObjectRef::new(PyObject::Type { name: "BufferedRWPair".to_string(), dict: Box::new(brp_dict), bases: vec![buf_cls.clone()], mro: vec![buf_cls.clone()] });
    d.insert("BufferedRWPair".to_string(), brp_cls.clone());
    let brnd_dict = HashMap::new(); let brnd_cls = PyObjectRef::new(PyObject::Type { name: "BufferedRandom".to_string(), dict: Box::new(brnd_dict), bases: vec![buf_cls.clone()], mro: vec![buf_cls.clone()] });
    d.insert("BufferedRandom".to_string(), brnd_cls.clone());

    // TextIOWrapper — stub type needed by io.py
    let mut tiw_dict = HashMap::new();
    tiw_dict.insert("read".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "read".to_string(), func: |_: &[PyObjectRef]| Ok(py_str("")) }));
    tiw_dict.insert("write".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "write".to_string(), func: |_: &[PyObjectRef]| Ok(py_none()) }));
    tiw_dict.insert("close".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "close".to_string(), func: |_: &[PyObjectRef]| Ok(py_none()) }));
    let tiw_cls = PyObjectRef::new(PyObject::Type { name: "TextIOWrapper".to_string(), dict: Box::new(tiw_dict), bases: vec![], mro: vec![] });
    d.insert("TextIOWrapper".to_string(), tiw_cls);

    d.insert("_WindowsConsoleIO".to_string(), py_str("_WindowsConsoleIO"));

    d
}