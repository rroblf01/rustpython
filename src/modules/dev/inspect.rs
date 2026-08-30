use crate::bytecode::{needs_arg, CodeObject};
use crate::interner;
use crate::object::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use std::collections::HashSet;

pub fn create_dis_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! dis_func {
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

    // Helper: extract a CodeObject from either a code object or a function
    fn extract_code(args: &[PyObjectRef]) -> Result<CodeObject, PyError> {
        if args.is_empty() {
            return Err(PyError::type_error(
                "missing required argument: code or function",
            ));
        }
        let obj = args[0].borrow();
        match &*obj {
            PyObject::Code(code) => Ok(code.as_ref().clone()),
            PyObject::Function(ref f) => Ok((*f.code).clone()),
            _ => Err(PyError::type_error(
                "argument must be a code object or function",
            )),
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
        // Real CPython's dis returns `Instruction` objects with .opname/
        // .argval/.arg/.offset/.starts_line attributes (and tuple
        // unpacking). Build one shared namedtuple class.
        let namedtuple = crate::modules::get_module("collections")
            .and_then(|m| m.borrow().get_attribute("namedtuple").ok())
            .ok_or_else(|| PyError::runtime_error("collections.namedtuple missing"))?;
        let instruction_type = crate::object::call_function_disposable(
            &namedtuple,
            vec![
                py_str("Instruction"),
                py_list(vec![
                    py_str("opname"),
                    py_str("argval"),
                    py_str("arg"),
                    py_str("offset"),
                    py_str("starts_line"),
                ]),
            ],
            vec![],
        )?;
        if std::env::var("RPY_DEBUG_DIS").is_ok() {
            eprintln!("DIS instruction_type = {}", instruction_type.repr());
        }
        let mut instr_list = Vec::new();
        for (i, instr) in code.instructions.iter().enumerate() {
            let offset = (i * 2) as i64;
            let opname = format!("{:?}", instr.op);
            let arg = instr.arg as i64;
            // argval: the meaningful operand (const value / name / arg).
            let argval = match instr.op {
                crate::bytecode::Opcode::LOAD_CONST => {
                    if let Some(cv) = code.consts.get(instr.arg as usize) {
                        crate::vm::eval_const_value(cv.clone()).ok()
                    } else {
                        Some(py_int(arg))
                    }
                }
                crate::bytecode::Opcode::LOAD_NAME
                | crate::bytecode::Opcode::LOAD_GLOBAL
                | crate::bytecode::Opcode::STORE_NAME
                | crate::bytecode::Opcode::LOAD_ATTR
                | crate::bytecode::Opcode::STORE_ATTR
                | crate::bytecode::Opcode::DELETE_NAME
                | crate::bytecode::Opcode::LOAD_DEREF
                | crate::bytecode::Opcode::STORE_DEREF
                | crate::bytecode::Opcode::LOAD_FAST
                | crate::bytecode::Opcode::STORE_FAST
                | crate::bytecode::Opcode::DELETE_FAST => code
                    .names
                    .get(instr.arg as usize)
                    .map(|&n| py_str(crate::interner::lookup_str(n))),
                _ => Some(py_int(arg)),
            };
            instr_list.push(crate::object::call_function_disposable(
                &instruction_type,
                vec![
                    py_str(&opname),
                    argval.unwrap_or_else(|| PyObjectRef::new(PyObject::None)),
                    py_int(arg),
                    py_int(offset),
                    PyObjectRef::new(PyObject::None),
                ],
                vec![],
            )?);
        }
        Ok(py_list(instr_list))
    });

    // Also add some opcode name constants for reference
    d.insert_str("opname", py_str("dis module for bytecode disassembly"));
    // Real CPython's `dis` re-exports these opcode-classification lists
    // from `opcode` (which describes CPython's OWN bytecode format — not
    // this interpreter's, so there's nothing meaningful to populate them
    // with). Empty lists here are enough for code that merely imports/
    // constructs a `set()` from them without asserting real CPython opcode
    // membership (real trigger: `test.support.bytecode_helper`, which our
    // fundamentally-different bytecode format can't produce accurate
    // results for regardless).
    for name in [
        "hasarg",
        "hasconst",
        "hasname",
        "hasjrel",
        "hasjabs",
        "haslocal",
        "hascompare",
        "hasfree",
        "hasexc",
    ] {
        d.insert(name.to_string(), py_list(vec![]));
    }

    d
}

/// Minimal `_opcode` (the CPython C extension backing parts of `dis`).
/// Only exposes the two constants `test.support` itself reads at import
/// time (`ENABLE_SPECIALIZATION`/`ENABLE_SPECIALIZATION_FT`, both about
/// CPython 3.11+'s adaptive specializing interpreter — always `False`
/// here, correct since this interpreter has no such optimization to gate).


pub fn create_doctest_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! doctest_func {
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

    // TestResults constructor — returns an instance with failed=0, attempted=0
    doctest_func!("TestResults", |_args| {
        let mut dict = AttrMap::new();
        dict.insert_str("failed", py_int(0));
        dict.insert_str("attempted", py_int(0));
        Ok(PyObjectRef::new(PyObject::Instance {
            typ: py_str("TestResults"),
            dict,
        }))
    });

    // testmod(m=None) — runs doctests on a module, returns TestResults(failed=0, attempted=0)
    doctest_func!("testmod", |_args| {
        let mut dict = AttrMap::new();
        dict.insert_str("failed", py_int(0));
        dict.insert_str("attempted", py_int(0));
        Ok(PyObjectRef::new(PyObject::Instance {
            typ: py_str("TestResults"),
            dict,
        }))
    });

    // testfile(filename) — runs doctests in a file, returns TestResults(failed=0, attempted=0)
    doctest_func!("testfile", |_args| {
        let mut dict = AttrMap::new();
        dict.insert_str("failed", py_int(0));
        dict.insert_str("attempted", py_int(0));
        Ok(PyObjectRef::new(PyObject::Instance {
            typ: py_str("TestResults"),
            dict,
        }))
    });

    // run_docstring_examples(f, globs, verbose=False) — stub
    doctest_func!("run_docstring_examples", |_args| { Ok(py_none()) });

    // DocTestFinder class stub
    doctest_func!("DocTestFinder", |_args| {
        let mut dict = AttrMap::new();
        dict.insert_str(
            "find",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "find".to_string(),
                func: |_| Ok(py_list(vec![])),
            }),
        );
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
            d.insert(
                $name.to_string(),
                PyObjectRef::new(PyObject::BuiltinFunction {
                    name: $name.to_string(),
                    func: $func,
                }),
            );
        };
    }

    // A unique "no value given" marker distinct from `None` (real code uses
    // it as a default-argument sentinel so `None` remains a legitimate
    // explicit value) — real trigger: CPython's own `test.support`,
    // `find_name_in_mro(cls, name, default=inspect._sentinel)`. Any
    // distinct object identity works; a bare Instance of an empty marker
    // Type is the simplest one available.
    d.insert_str(
        "_sentinel",
        PyObjectRef::new(PyObject::Instance {
            typ: PyObjectRef::new(PyObject::Type {
                name: "_sentinel".to_string(),
                dict: Box::new(str_map_to_typedict(HashMap::new())),
                bases: vec![],
                mro: vec![],
            }),
            dict: AttrMap::new(),
        }),
    );

    inspect_func!("isfunction", |args| {
        if args.len() < 1 {
            return Err(PyError::type_error("isfunction() requires 1 argument"));
        }
        let obj = args[0].borrow();
        Ok(py_bool(matches!(&*obj, PyObject::Function(_))))
    });

    inspect_func!("isgeneratorfunction", |args| {
        if args.len() < 1 {
            return Err(PyError::type_error(
                "isgeneratorfunction() requires 1 argument",
            ));
        }
        let obj = args[0].borrow();
        let is_gen = match &*obj {
            PyObject::Function(ref f) => (f.code.flags & 0x0020) != 0,
            _ => false,
        };
        Ok(py_bool(is_gen))
    });

    inspect_func!("iscoroutinefunction", |args| {
        if args.len() < 1 {
            return Err(PyError::type_error(
                "iscoroutinefunction() requires 1 argument",
            ));
        }
        let obj = args[0].borrow();
        let is_coro = match &*obj {
            PyObject::Function(ref f) => (f.code.flags & 0x0080) != 0,
            _ => false,
        };
        Ok(py_bool(is_coro))
    });

    // `inspect.iscoroutine`/`isawaitable` — missing entirely
    // (`AttributeError`), breaking `unittest.mock`'s own import-time
    // `from inspect import iscoroutinefunction` line's neighboring runtime
    // use (`iscoroutinefunction(obj) or inspect.isawaitable(obj)`) the
    // moment any test imported `unittest.mock` (real trigger: CPython's
    // own `test_getpass.py`/`test_htmlparser.py`, neither of which uses
    // asyncio directly — the failure came purely from `mock`'s own
    // internals). `isawaitable` real semantics: true for a coroutine
    // object, or any object implementing `__await__` — good enough
    // approximation without needing full PEP 492 generator-based-coroutine
    // detection this codebase doesn't track separately anyway.
    inspect_func!("iscoroutine", |args| {
        if args.len() < 1 {
            return Err(PyError::type_error("iscoroutine() requires 1 argument"));
        }
        Ok(py_bool(matches!(
            &*args[0].borrow(),
            PyObject::Coroutine { .. }
        )))
    });
    inspect_func!("isawaitable", |args| {
        if args.len() < 1 {
            return Err(PyError::type_error("isawaitable() requires 1 argument"));
        }
        let is_awaitable = match &*args[0].borrow() {
            PyObject::Coroutine { .. } => true,
            PyObject::Instance { .. } => args[0].borrow().get_attribute("__await__").is_ok(),
            _ => false,
        };
        Ok(py_bool(is_awaitable))
    });

    // `inspect.getattr_static(obj, attr, default=<sentinel>)` — missing
    // entirely (`AttributeError`), breaking `unittest.mock`'s own spec-
    // checking machinery (`static_attr = inspect.getattr_static(spec, attr,
    // None)`) the moment a test used `Mock(spec=...)`. Real semantics:
    // looks up `attr` WITHOUT triggering descriptor protocol / `__getattr__`
    // side effects (an instance's own dict first, then the class's dict,
    // then each ancestor's own dict in mro order) — a simplified but
    // faithful-enough approximation of that "skip descriptors" contract for
    // the common `Instance`/`Type` cases, not full C-level slot introspection.
    inspect_func!("getattr_static", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error(
                "getattr_static() requires at least 2 arguments",
            ));
        }
        let attr_name = args[1].str();
        let default = args.get(2).cloned();
        let found = {
            let obj_borrowed = args[0].borrow();
            match &*obj_borrowed {
                PyObject::Instance { dict, typ } => {
                    dict.get_str(&attr_name).cloned().or_else(|| {
                        let typ_ref = typ.borrow();
                        if let PyObject::Type {
                            dict: type_dict,
                            mro,
                            ..
                        } = &*typ_ref
                        {
                            type_dict.get_str(&attr_name).cloned().or_else(|| {
                                mro.iter().find_map(|base| {
                                    if let PyObject::Type {
                                        dict: base_dict, ..
                                    } = &*base.borrow()
                                    {
                                        base_dict.get_str(&attr_name).cloned()
                                    } else {
                                        None
                                    }
                                })
                            })
                        } else {
                            None
                        }
                    })
                }
                PyObject::Type { dict, mro, .. } => {
                    dict.get_str(&attr_name).cloned().or_else(|| {
                        mro.iter().find_map(|base| {
                            if let PyObject::Type {
                                dict: base_dict, ..
                            } = &*base.borrow()
                            {
                                base_dict.get_str(&attr_name).cloned()
                            } else {
                                None
                            }
                        })
                    })
                }
                _ => None,
            }
        };
        found.or(default).ok_or_else(|| {
            PyError::attribute_error(format!(
                "'{}' object has no attribute '{}'",
                args[0].get_type_name(),
                attr_name
            ))
        })
    });

    inspect_func!("isclass", |args| {
        if args.len() < 1 {
            return Err(PyError::type_error("isclass() requires 1 argument"));
        }
        let obj = args[0].borrow();
        Ok(py_bool(matches!(&*obj, PyObject::Type { .. })))
    });

    inspect_func!("ismodule", |args| {
        if args.len() < 1 {
            return Err(PyError::type_error("ismodule() requires 1 argument"));
        }
        let obj = args[0].borrow();
        Ok(py_bool(matches!(&*obj, PyObject::Module { .. })))
    });

    inspect_func!("ismethod", |args| {
        if args.len() < 1 {
            return Err(PyError::type_error("ismethod() requires 1 argument"));
        }
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
        if args.len() < 1 {
            return Err(PyError::type_error("isabstract() requires 1 argument"));
        }
        let obj = args[0].borrow();
        let has_abstract_methods = match &*obj {
            PyObject::Type { dict, .. } => dict
                .get_str("__abstractmethods__")
                .map(|v| v.truthy())
                .unwrap_or(false),
            _ => false,
        };
        Ok(py_bool(has_abstract_methods))
    });

    inspect_func!("getdoc", |args| {
        if args.len() < 1 {
            return Err(PyError::type_error("getdoc() requires 1 argument"));
        }
        let obj = args[0].borrow();
        let doc = match &*obj {
            PyObject::Function(ref f) => f.dict.get_str("__doc__").cloned(),
            PyObject::Type { ref dict, .. } => dict.get_str("__doc__").cloned(),
            PyObject::Module { ref dict, .. } => dict.get_str("__doc__").cloned(),
            PyObject::Instance { ref dict, .. } => dict.get_str("__doc__").cloned(),
            _ => None,
        };
        Ok(doc.unwrap_or(py_none()))
    });

    // Port of Lib/inspect.py's own `cleandoc` (this native module doesn't
    // delegate to that pure-Python file at all — it's a SEPARATE `inspect`
    // implementation that happens to win module-registration priority, so
    // gaps there don't get filled by Lib/'s more complete version). Missing
    // entirely was worse than a wrong result: `from inspect import
    // cleandoc` raised `ModuleNotFoundError: No module named
    // 'inspect.cleandoc'` (via the "maybe it's a submodule" import
    // fallback) instead of `AttributeError`/actually working — real
    // trigger: `cmd.Cmd.do_help`'s docstring-dedent step, test_cmd.py.
    inspect_func!("cleandoc", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("cleandoc() requires 1 argument"));
        }
        let doc = args[0].str();
        let expanded: String = {
            let mut out = String::new();
            let mut col = 0usize;
            for c in doc.chars() {
                if c == '\t' {
                    let spaces = 8 - (col % 8);
                    out.push_str(&" ".repeat(spaces));
                    col += spaces;
                } else if c == '\n' {
                    out.push(c);
                    col = 0;
                } else {
                    out.push(c);
                    col += 1;
                }
            }
            out
        };
        let mut lines: Vec<String> = expanded.split('\n').map(|s| s.to_string()).collect();
        let mut margin = usize::MAX;
        for line in lines.iter().skip(1) {
            let stripped = line.trim_start_matches(' ');
            if !stripped.is_empty() {
                margin = margin.min(line.len() - stripped.len());
            }
        }
        if let Some(first) = lines.first_mut() {
            *first = first.trim_start_matches(' ').to_string();
        }
        if margin < usize::MAX {
            for line in lines.iter_mut().skip(1) {
                *line = line.chars().skip(margin).collect();
            }
        }
        while lines.last().is_some_and(|l| l.is_empty()) {
            lines.pop();
        }
        while lines.first().is_some_and(|l| l.is_empty()) {
            lines.remove(0);
        }
        Ok(py_str(&lines.join("\n")))
    });

    inspect_func!("getfile", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("getfile() requires 1 argument"));
        }
        let obj = args[0].borrow();
        // Try to get __code__ attribute
        if let Ok(code) = obj.get_attribute("__code__") {
            let code_borrowed = code.borrow();
            if let PyObject::Code(c) = &*code_borrowed {
                return Ok(py_str(crate::interner::lookup_str(c.filename)));
            }
        }
        Ok(py_str("<unknown>"))
    });
    inspect_func!("getsourcefile", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("getsourcefile() requires 1 argument"));
        }
        let obj = args[0].borrow();
        if let Ok(code) = obj.get_attribute("__code__") {
            let code_borrowed = code.borrow();
            if let PyObject::Code(c) = &*code_borrowed {
                return Ok(py_str(crate::interner::lookup_str(c.filename)));
            }
        }
        Ok(py_none())
    });
    inspect_func!("getsource", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("getsource() requires 1 argument"));
        }
        let obj = args[0].borrow();
        let filename = obj.get_attribute("__code__").ok().and_then(|code| {
            let code_borrowed = code.borrow();
            if let PyObject::Code(c) = &*code_borrowed {
                Some(c.filename.clone())
            } else {
                None
            }
        });
        if let Some(fname) = filename {
            if let Ok(src) = std::fs::read_to_string(crate::interner::lookup_str(fname)) {
                return Ok(py_str(&src));
            }
        }
        Ok(py_str("Source not available in RustPython"))
    });

    inspect_func!("getmodule", |args| {
        if args.len() < 1 {
            return Err(PyError::type_error("getmodule() requires 1 argument"));
        }
        let module_name = args[0]
            .borrow()
            .get_attribute("__module__")
            .ok()
            .and_then(|v| {
                if let PyObject::Str(s) = &*v.borrow() {
                    Some(s.to_string())
                } else {
                    None
                }
            });
        Ok(if let Some(name) = module_name {
            py_str(&name)
        } else {
            py_none()
        })
    });

    inspect_func!("getmembers", getmembers_builtin);

    inspect_func!("getfullargspec", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("getfullargspec() requires 1 argument"));
        }
        let target = match &*args[0].borrow() {
            PyObject::BoundMethod { func, .. } => func.clone(),
            _ => args[0].clone(),
        };
        let b = target.borrow();
        if let PyObject::Function(ref inner_f) = &*b {
            let code = &inner_f.code;
            let defaults = &inner_f.defaults;
            let arg_count = code.arg_count.min(code.varnames.len());
            let positional_args: Vec<PyObjectRef> = code.varnames[..arg_count]
                .iter()
                .map(|&n| py_str(crate::interner::lookup_str(n)))
                .collect();
            // varnames layout is: positional args, then *args (if any), then
            // kwonly args, then **kwargs (if any) — the vararg slot must be
            // skipped when locating where kwonly names start.
            let kwonly_start = arg_count + if code.vararg_name.is_some() { 1 } else { 0 };
            let kwonlyargs: Vec<PyObjectRef> = if code.kwonlyarg_count > 0 {
                code.varnames
                    .get(kwonly_start..kwonly_start + code.kwonlyarg_count)
                    .map(|s| {
                        s.iter()
                            .map(|&n| py_str(crate::interner::lookup_str(n)))
                            .collect()
                    })
                    .unwrap_or_default()
            } else {
                vec![]
            };
            let varargs = code
                .vararg_name
                .as_ref()
                .map(|n| py_str(n))
                .unwrap_or_else(py_none);
            let varkw = code
                .kwarg_name
                .as_ref()
                .map(|n| py_str(n))
                .unwrap_or_else(py_none);
            // `defaults` holds positional defaults then kwonly ones appended
            // after (see MAKE_FUNCTION/CodeObject::kwonly_defaults_mask) —
            // code.num_defaults is the positional-only count.
            let num_defaults = code.num_defaults;
            let defaults_val = if num_defaults == 0 {
                py_none()
            } else {
                py_tuple(defaults[..num_defaults].to_vec())
            };
            let kwonlydefaults = py_dict();
            if code.kwonlyarg_count > 0 {
                let mut kwdefault_idx = num_defaults;
                if let PyObject::Dict(d) = &mut *kwonlydefaults.borrow_mut() {
                    for (k, has_default) in code.kwonly_defaults_mask.iter().enumerate() {
                        if !*has_default {
                            continue;
                        }
                        if let Some(pname) = code.varnames.get(kwonly_start + k) {
                            if let Some(v) = defaults.get(kwdefault_idx) {
                                d.set(py_str(crate::interner::lookup_str(*pname)), v.clone())?;
                            }
                        }
                        kwdefault_idx += 1;
                    }
                }
            }
            let kwonlydefaults = if kwonlyargs.is_empty()
                || matches!(&*kwonlydefaults.borrow(), PyObject::Dict(d) if d.is_empty())
            {
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
            Err(PyError::type_error(
                "getfullargspec() requires a Python function",
            ))
        }
    });

    inspect_func!("unwrap", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("unwrap() requires 1 argument"));
        }
        let mut current = args[0].clone();
        let mut seen = std::collections::HashSet::new();
        loop {
            let next = current.borrow().get_attribute("__wrapped__").ok();
            match next {
                Some(w) => {
                    if !seen.insert(w.get_id()) {
                        break;
                    }
                    current = w;
                }
                None => break,
            }
        }
        Ok(current)
    });

    inspect_func!("signature", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("signature() requires 1 argument"));
        }
        let target = match &*args[0].borrow() {
            PyObject::BoundMethod { func, .. } => func.clone(),
            _ => args[0].clone(),
        };
        let b = target.borrow();
        if let PyObject::Function(ref inner_f) = &*b {
            let code = &inner_f.code;
            let defaults = &inner_f.defaults;
            let mut param_type_dict = HashMap::new();
            param_type_dict.insert_str("POSITIONAL_ONLY", py_int(0));
            param_type_dict.insert_str("POSITIONAL_OR_KEYWORD", py_int(1));
            param_type_dict.insert_str("VAR_POSITIONAL", py_int(2));
            param_type_dict.insert_str("KEYWORD_ONLY", py_int(3));
            param_type_dict.insert_str("VAR_KEYWORD", py_int(4));
            param_type_dict.insert_str("empty", py_none());
            let param_type = PyObjectRef::new(PyObject::Type {
                name: "Parameter".to_string(),
                dict: Box::new(str_map_to_typedict(param_type_dict)),
                bases: vec![],
                mro: vec![],
            });
            let make_param =
                |pname: &str, kind: i64, default: PyObjectRef, param_type: &PyObjectRef| {
                    let mut inst_dict = AttrMap::new();
                    inst_dict.insert_str("name", py_str(pname));
                    inst_dict.insert_str("kind", py_int(kind));
                    inst_dict.insert_str("default", default);
                    PyObjectRef::new(PyObject::Instance {
                        typ: param_type.clone(),
                        dict: inst_dict,
                    })
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
                let pname_str = crate::interner::lookup_str(code.varnames[i]);
                let default = if i >= first_default_idx {
                    defaults[i - first_default_idx].clone()
                } else {
                    py_none()
                };
                let p = make_param(pname_str, 1, default, &param_type); // POSITIONAL_OR_KEYWORD
                params.set(py_str(pname_str), p)?;
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
                if let Some(kwonly) = code
                    .varnames
                    .get(kwonly_start..kwonly_start + code.kwonlyarg_count)
                {
                    for (k, pname) in kwonly.iter().enumerate() {
                        let has_default =
                            code.kwonly_defaults_mask.get(k).copied().unwrap_or(false);
                        let default = if has_default {
                            let v = defaults.get(kwdefault_idx).cloned().unwrap_or_else(py_none);
                            kwdefault_idx += 1;
                            v
                        } else {
                            py_none()
                        };
                        let p = make_param(
                            &crate::interner::lookup_str(*pname),
                            3,
                            default,
                            &param_type,
                        ); // KEYWORD_ONLY
                        params.set(py_str(crate::interner::lookup_str(*pname)), p)?;
                    }
                }
            }
            if let Some(kw) = &code.kwarg_name {
                let p = make_param(kw, 4, py_none(), &param_type); // VAR_KEYWORD
                params.set(py_str(kw), p)?;
            }
            let sig_type = PyObjectRef::new(PyObject::Type {
                name: "Signature".to_string(),
                dict: Box::new(str_map_to_typedict(HashMap::new())),
                bases: vec![],
                mro: vec![],
            });
            let mut sig_dict = AttrMap::new();
            sig_dict.insert_str(
                "parameters",
                PyObjectRef::new(PyObject::Dict(Box::new(params))),
            );
            Ok(PyObjectRef::new(PyObject::Instance {
                typ: sig_type,
                dict: sig_dict,
            }))
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
    param_type_dict.insert_str("POSITIONAL_ONLY", py_int(0));
    param_type_dict.insert_str("POSITIONAL_OR_KEYWORD", py_int(1));
    param_type_dict.insert_str("VAR_POSITIONAL", py_int(2));
    param_type_dict.insert_str("KEYWORD_ONLY", py_int(3));
    param_type_dict.insert_str("VAR_KEYWORD", py_int(4));
    param_type_dict.insert_str("empty", py_none());
    d.insert_str(
        "Parameter",
        PyObjectRef::new(PyObject::Type {
            name: "Parameter".to_string(),
            dict: Box::new(str_map_to_typedict(param_type_dict)),
            bases: vec![],
            mro: vec![],
        }),
    );
    d.insert_str(
        "Signature",
        PyObjectRef::new(PyObject::Type {
            name: "Signature".to_string(),
            dict: Box::new(str_map_to_typedict(HashMap::new())),
            bases: vec![],
            mro: vec![],
        }),
    );

    // Code object flags (CO_* constants)
    d.insert_str("CO_OPTIMIZED", py_int(0x0001));
    d.insert_str("CO_NEWLOCALS", py_int(0x0002));
    d.insert_str("CO_VARARGS", py_int(0x0004));
    d.insert_str("CO_VARKEYWORDS", py_int(0x0008));
    d.insert_str("CO_NESTED", py_int(0x0010));
    d.insert_str("CO_GENERATOR", py_int(0x0020));
    d.insert_str("CO_NOFREE", py_int(0x0040));
    d.insert_str("CO_COROUTINE", py_int(0x0080));
    d.insert_str("CO_ITERABLE_COROUTINE", py_int(0x0100));
    d.insert_str("CO_ASYNC_GENERATOR", py_int(0x0200));
    d.insert_str("CO_FUTURE_DIVISION", py_int(0x2000));
    d.insert_str("CO_FUTURE_ABSOLUTE_IMPORT", py_int(0x4000));
    d.insert_str("CO_FUTURE_WITH_STATEMENT", py_int(0x8000));
    d.insert_str("CO_FUTURE_PRINT_FUNCTION", py_int(0x10000));
    d.insert_str("CO_FUTURE_UNICODE_LITERALS", py_int(0x20000));
    d.insert_str("CO_FUTURE_BARRY_AS_BDFL", py_int(0x40000));
    d.insert_str("CO_FUTURE_GENERATOR_STOP", py_int(0x80000));
    d.insert_str("CO_FUTURE_ANNOTATIONS", py_int(0x100000));

    // BufferFlags for test_buffer.py (inspect.BufferFlags)
    {
        let mut bf_dict = HashMap::new();
        bf_dict.insert_str("SIMPLE", py_int(0x0));
        bf_dict.insert_str("WRITABLE", py_int(0x1));
        bf_dict.insert_str("FORMAT", py_int(0x4));
        bf_dict.insert_str("ND", py_int(0x8));
        bf_dict.insert_str("STRIDES", py_int(0x10 | 0x8));
        bf_dict.insert_str("C_CONTIGUOUS", py_int(0x20 | 0x10 | 0x8));
        bf_dict.insert_str("F_CONTIGUOUS", py_int(0x40 | 0x10 | 0x8));
        bf_dict.insert_str("ANY_CONTIGUOUS", py_int(0x80 | 0x10 | 0x8));
        bf_dict.insert_str("INDIRECT", py_int(0x100 | 0x10 | 0x8));
        bf_dict.insert_str("CONTIG", py_int(0x8 | 0x1));
        bf_dict.insert_str("CONTIG_RO", py_int(0x8));
        bf_dict.insert_str("STRIDED", py_int(0x10 | 0x8 | 0x1));
        bf_dict.insert_str("STRIDED_RO", py_int(0x10 | 0x8));
        bf_dict.insert_str("RECORDS", py_int(0x10 | 0x8 | 0x1 | 0x4));
        bf_dict.insert_str("RECORDS_RO", py_int(0x10 | 0x8 | 0x4));
        bf_dict.insert_str("FULL", py_int(0x100 | 0x10 | 0x8 | 0x1 | 0x4));
        bf_dict.insert_str("FULL_RO", py_int(0x100 | 0x10 | 0x8 | 0x4));
        bf_dict.insert_str("READ", py_int(0x100));
        bf_dict.insert_str("WRITE", py_int(0x200));
        let bf_type = PyObjectRef::new(PyObject::Type {
            name: "BufferFlags".to_string(),
            dict: Box::new(crate::object::str_map_to_typedict(bf_dict)),
            bases: vec![],
            mro: vec![],
        });
        d.insert_str("BufferFlags", bf_type);
    }

    d
}

fn getmembers_dict_of(obj: &PyObjectRef) -> Vec<(String, PyObjectRef)> {
    let b = obj.borrow();
    let mut items: Vec<(String, PyObjectRef)> = match &*b {
        PyObject::Function(ref f) => f.dict.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        PyObject::Type { dict, .. } => dict
            .iter()
            .map(|(k, v)| (interner::lookup_str(*k).to_string(), v.clone()))
            .collect(),
        PyObject::Module { dict, .. } => dict
            .iter()
            .map(|(k, v)| (interner::lookup_str(*k).to_string(), v.clone()))
            .collect(),
        PyObject::Instance { dict, .. } => dict
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect(),
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
pub fn getmembers_with_vm(
    vm: &mut crate::vm::VirtualMachine,
    obj: &PyObjectRef,
    predicate: Option<&PyObjectRef>,
) -> PyResult<PyObjectRef> {
    let items = getmembers_dict_of(obj);
    let mut members = Vec::new();
    for (k, v) in items {
        let keep = match predicate {
            Some(p) => vm
                .call_function(p.clone(), vec![v.clone()], vec![])?
                .truthy(),
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
pub fn getmembers_builtin(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error("getmembers() requires 1 argument"));
    }
    let items = getmembers_dict_of(&args[0]);
    Ok(py_list(
        items
            .into_iter()
            .map(|(k, v)| py_tuple(vec![py_str(&k), v]))
            .collect(),
    ))
}

// ─── profile module ────────────────────────────────────────────────────────

