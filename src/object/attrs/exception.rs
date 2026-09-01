// Auto-extracted from src/object/attrs/mod.rs lines 1421-1784
use crate::object::*;
use super::*;
use crate::interner;
use std::rc::Rc;

pub(crate) fn get(o: &PyObject, name: &str) -> PyResult<PyObjectRef> {
    match o {
            PyObject::Exception {
                typ,
                args,
                cause,
                suppress_context,
                context,
                traceback,
                extra,
            } => {
                match name {
                    "__name__" => Ok(py_str(typ)),
                    "args" => Ok(py_tuple(args.clone())),
                    // `StopIteration.value` (and StopAsyncIteration) — the
                    // value a generator/coroutine returned (real code: a
                    // driver does `coro.send(None)` and reads `e.value`).
                    // Real CPython: StopIteration(value).value == value.
                    "value" if typ == "StopIteration" || typ == "StopAsyncIteration" => {
                        if args.len() == 1 {
                            Ok(args[0].clone())
                        } else if args.is_empty() {
                            Ok(py_none())
                        } else {
                            Ok(py_tuple(args.clone()))
                        }
                    }
                    // `lineno`/`offset` — a real SyntaxError carries its
                    // source position (test.support's check_syntax_error
                    // asserts both are not None). The parser's error
                    // messages embed "L<line>:<col>:" as a prefix; parse it
                    // out lazily. Defaults to None for non-syntax errors.
                    "lineno" | "offset" => {
                        let want_lineno = name == "lineno";
                        // A ctor-set SyntaxError location tuple wins over the
                        // lazy "L<line>:<col>:" parsing below.
                        if typ == "SyntaxError" {
                            if let Some(extra) = extra {
                                if let Some(v) = extra.get(name) {
                                    return Ok(v.clone());
                                }
                            }
                        }
                        let parsed = args.first().and_then(|a| {
                            let s = a.str();
                            if let Some(rest) = s.strip_prefix('L') {
                                let (ln, rest) = rest.split_once(':')?;
                                let (col, _rest) = rest.split_once(':')?;
                                let line = ln.parse::<i64>().ok()?;
                                let offset = col.parse::<i64>().ok()?;
                                Some((line, offset))
                            } else {
                                None
                            }
                        });
                        match parsed {
                            Some((line, offset)) => {
                                Ok(py_int(if want_lineno { line } else { offset }))
                            }
                            None => Ok(py_none()),
                        }
                    }
                    // `encoding`/`object`/`start`/`end`/`reason` — the
                    // UnicodeError family's five positional args
                    // (UnicodeEncodeError('utf-8', obj, start, end, reason));
                    // codec error-handler functions (backslashreplace_errors
                    // etc.) read these.
                    "encoding" | "object" | "start" | "end" | "reason"
                        if typ == "UnicodeError"
                            || typ == "UnicodeEncodeError"
                            || typ == "UnicodeDecodeError"
                            || typ == "UnicodeTranslateError" =>
                    {
                        let idx = match name {
                            "encoding" => 0,
                            "object" => 1,
                            "start" => 2,
                            "end" => 3,
                            _ => 4,
                        };
                        match args.get(idx) {
                            Some(v) => Ok(v.clone()),
                            None => Ok(py_none()),
                        }
                    }
                    // `__str__`/`__repr__` — real exceptions always expose
                    // both (test_baseexception's verify_instance_interface
                    // asserts `args`/`__str__`/`__repr__` on EVERY builtin
                    // exception instance). CPython: str(exc) joins str(args)
                    // (empty args -> empty string); repr is `TypeName(args)`.
                    "__str__" => {
                        let parts: Vec<String> = args.iter().map(|a| a.str()).collect();
                        Ok(py_str(&parts.join(", ")))
                    }
                    "__repr__" => {
                        let parts: Vec<String> = args.iter().map(|a| a.repr()).collect();
                        Ok(py_str(&format!("{}({})", typ, parts.join(", "))))
                    }
                    "__cause__" => match cause {
                        Some(cause_exc) => Ok(cause_exc.clone()),
                        None => Ok(py_none()),
                    },
                    "__context__" => match context {
                        Some(ctx_exc) => Ok(ctx_exc.clone()),
                        None => Ok(py_none()),
                    },
                    // PEP 3134 implicit exception chaining/traceback
                    // attributes every real exception instance carries
                    // (defaulting to `None`/`False`) — this interpreter
                    // doesn't implement implicit `__context__` capture (an
                    // exception raised while another is being handled)
                    // or a real traceback OBJECT, but code that merely
                    // checks these are present/None (real trigger:
                    // `unittest`'s own `TestResult._clean_tracebacks`,
                    // `for c in (value.__cause__, value.__context__): if c
                    // is not None: ...`) previously raised AttributeError
                    // just from the attribute not existing at all.
                    "__traceback__" => match traceback {
                        Some(tb) => Ok(tb.clone()),
                        None => Ok(py_none()),
                    },
                    "__suppress_context__" => Ok(py_bool(*suppress_context)),
                    "__notes__" => {
                        if let Some(extra) = extra {
                            if let Some(v) = extra.get("__notes__") {
                                return Ok(v.clone());
                            }
                        }
                        Ok(py_list(vec![]))
                    }
                    // Per-instance attributes (BaseException.__dict__): the
                    // constructor's keyword args (`AttributeError('x',
                    // name=..., obj=...)`) and anything assigned by user
                    // code. `__dict__` returns a copy; name/obj are the
                    // AttributeError-specific ones CPython's test_exceptions
                    // asserts.
                    "__dict__" => {
                        let mut d = crate::object::PyDict::new();
                        if let Some(extra) = extra {
                            for (k, v) in extra.iter() {
                                let _ = d.set(py_str(k), v.clone());
                            }
                        }
                        Ok(PyObjectRef::new(PyObject::Dict(Box::new(d))))
                    }
                    // `BaseException.__setstate__(state)` — inherited by
                    // every exception, used by pickle to restore extra
                    // instance attributes on unpickling. Real semantics:
                    // `None` is a no-op, a `dict` merges into `__dict__`,
                    // anything else raises `TypeError` (found via CPython's
                    // own `test_exceptions.py::test_invalid_setstate`, which
                    // checks exactly this error case). Exceptions here have
                    // no generic attribute-dict storage the way `Instance`
                    // does, so a valid dict argument is accepted (matching
                    // real behavior/not raising) but not actually persisted
                    // — a narrower, deliberate limitation, not the gap this
                    // fix targets.
                    "__setstate__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__setstate__".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "__setstate__() takes exactly one argument",
                                ));
                            }
                            match &*args[1].borrow() {
                                PyObject::None => Ok(py_none()),
                                PyObject::Dict(_) => {
                                    // Merge the state dict into the exception's
                                    // per-instance attrs (BaseException
                                    // `__dict__`), with the special `args` key
                                    // REPLACING the exception's args tuple —
                                    // pickle round-trips and
                                    // `e.__setstate__({'a': 1, 'args': (...)})`
                                    // work (test_exceptions::test_setstate).
                                    let mut m = std::collections::HashMap::new();
                                    if let PyObject::Dict(d) = &*args[1].borrow() {
                                        for (k, v) in d.iter() {
                                            let key = match &*k.borrow() {
                                                PyObject::Str(s) => s.to_string(),
                                                _ => continue,
                                            };
                                            m.insert(key, v.clone());
                                        }
                                    }
                                    let new_args = m.remove("args");
                                    if let PyObject::Exception { args, extra, .. } =
                                        &mut *args[0].borrow_mut()
                                    {
                                        if let Some(na) = new_args {
                                            let is_tuple =
                                                matches!(&*na.borrow(), PyObject::Tuple(_));
                                            let cloned = if is_tuple {
                                                match &*na.borrow() {
                                                    PyObject::Tuple(t) => t.clone(),
                                                    _ => unreachable!(),
                                                }
                                            } else {
                                                vec![na.clone()]
                                            };
                                            *args = cloned;
                                        }
                                        if !m.is_empty() {
                                            let store = extra.get_or_insert_with(|| {
                                                std::collections::HashMap::new()
                                            });
                                            for (k, v) in m {
                                                store.insert(k, v);
                                            }
                                        }
                                    }
                                    Ok(py_none())
                                }
                                _ => Err(PyError::type_error("state is not a dictionary")),
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "add_note" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "add_note".to_string(),
                        func: |_args| Ok(py_none()),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "with_traceback" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "with_traceback".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "with_traceback() takes exactly one argument",
                                ));
                            }
                            // Store the traceback so `raise X().with_traceback(tb)`
                            // yields `X.__traceback__` chaining tb (the RAISE
                            // unwind prepends the current frame's own node).
                            args[0]
                                .borrow_mut()
                                .set_attribute("__traceback__", args[1].clone())?;
                            Ok(args[0].clone())
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    // `e.__init__(*args)` — re-initializes the exception:
                    // replaces `.args` and resets per-instance attrs
                    // (test_reset_attributes: `exc.__init__()` clears
                    // msg/name/path). Returns None like object.__init__.
                    "__init__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__init__".to_string(),
                        func: |args| {
                            if let PyObject::Exception { args: a, extra, .. } =
                                &mut *args[0].borrow_mut()
                            {
                                *a = args.get(1..).unwrap_or(&[]).to_vec();
                                *extra = None;
                            }
                            Ok(py_none())
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    // `SyntaxError`'s extra attributes (`filename`/`lineno`/
                    // `offset`/`text`/`end_lineno`/`end_offset`) — this
                    // interpreter's own `syntax_error()` constructor
                    // (`errors.rs`) doesn't thread real source-location data
                    // through from the parser/compiler at all, so these
                    // can't carry genuine values yet — but real Python code
                    // that merely reads them (real trigger: CPython's own
                    // `test_exceptions.py`) previously got `AttributeError`
                    // instead of `None`, which is what real CPython itself
                    // returns for a `SyntaxError` constructed without the
                    // extra positional-args tuple. Gated to `SyntaxError`
                    // specifically — a plain `Exception`/`ValueError`/etc.
                    // genuinely has no such attributes in real Python either.
                    // `SyntaxError`'s location attributes come from the
                    // ctor's 6-tuple (`msg`, `filename`, `lineno`, `offset`,
                    // `text`, `end_lineno`, `end_offset`); reading them
                    // falls back to None when never set. `msg` additionally
                    // defaults to the first positional arg (`SyntaxError
                    // ('msgStr')` -> `.msg == 'msgStr'`).
                    "msg"
                    | "filename"
                    | "lineno"
                    | "offset"
                    | "text"
                    | "end_lineno"
                    | "end_offset"
                    | "print_file_and_line"
                        if typ == "SyntaxError" =>
                    {
                        if let Some(extra) = extra {
                            if let Some(v) = extra.get(name) {
                                return Ok(v.clone());
                            }
                        }
                        if name == "msg" {
                            if let Some(first) = args.first() {
                                return Ok(first.clone());
                            }
                        }
                        Ok(py_none())
                    }
                    // `AttributeError.name`/`.obj` default to None when not
                    // set by the constructor or getattr machinery.
                    "name" | "obj" if typ == "AttributeError" => {
                        if let Some(extra) = extra {
                            if let Some(v) = extra.get(name) {
                                return Ok(v.clone());
                            }
                        }
                        Ok(py_none())
                    }
                    // `ImportError.name`/`.path` (ctor kwargs, default None)
                    // and `.msg` (alias for args[0]).
                    "name" | "path" if typ == "ImportError" => {
                        if let Some(extra) = extra {
                            if let Some(v) = extra.get(name) {
                                return Ok(v.clone());
                            }
                        }
                        Ok(py_none())
                    }
                    "msg" if typ == "ImportError" => {
                        if let Some(first) = args.first() {
                            return Ok(first.clone());
                        }
                        Ok(py_none())
                    }
                    // `OSError.errno`/`.strerror`/`.filename`/`.filename2`
                    // (derived from the ctor's positional args).
                    "errno" | "strerror" | "filename" | "filename2"
                        if typ == "OSError" || typ == "EnvironmentError" =>
                    {
                        if let Some(extra) = extra {
                            if let Some(v) = extra.get(name) {
                                return Ok(v.clone());
                            }
                        }
                        Ok(py_none())
                    }
                    // `SystemExit.code` — args[0] when present, else None.
                    "code" if typ == "SystemExit" => {
                        Ok(args.first().cloned().unwrap_or_else(py_none))
                    }
                    // `NameError.name` — the undefined name (set by the VM's
                    // LOAD_NAME path), default None.
                    "name" if typ == "NameError" || typ == "UnboundLocalError" => {
                        if let Some(extra) = extra {
                            if let Some(v) = extra.get(name) {
                                return Ok(v.clone());
                            }
                        }
                        Ok(py_none())
                    }
                    _ => {
                        // Per-instance extras (BaseException.__dict__) —
                        // e.g. `AttributeError('x', name='carry').name`.
                        if let Some(extra) = extra {
                            if let Some(v) = extra.get(name) {
                                return Ok(v.clone());
                            }
                        }
                        Err(PyError::attribute_error(format!(
                            "'{}' object has no attribute '{}'",
                            typ, name
                        )))
                    }
                }
            }
        _ => unreachable!("{} called with non-{} object", stringify!(get), o.type_name()),
    }
}
