// Auto-extracted from src/object/attrs/mod.rs lines 1851-1898
use super::*;
use crate::interner;
use crate::object::*;

fn group_message(args: &[PyObjectRef]) -> String {
    args.first().map(|a| a.str()).unwrap_or_default()
}

/// True when every child is an `Exception` (not a `BaseException` that
/// escapes `Exception`), which is what lets a `BaseExceptionGroup` be
/// re-created as the narrower `ExceptionGroup` (CPython does exactly this
/// in `split`/`derive`).
fn all_are_exception_class(exceptions: &[PyObjectRef]) -> bool {
    exceptions.iter().all(|child| {
        let name = match &*child.borrow() {
            PyObject::Exception { typ, .. } | PyObject::ExceptionGroup { typ, .. } => typ.clone(),
            _ => return false,
        };
        crate::vm::util::is_exception_subclass(&name, "Exception")
    })
}

/// Rebuild a group with the SAME class and message but different children —
/// `eg.derive(excs)`, and what the `except*` split in `vm/op_exc.rs` needs
/// (it must refresh `args` too: CPython's derived group carries the NEW
/// children list in `args[1]`, which is what `repr()` prints). Includes the
/// BaseExceptionGroup -> ExceptionGroup narrowing CPython applies when the
/// new children are all `Exception` subclasses.
pub(crate) fn derive_group_from_args(
    typ: &str,
    args: &[PyObjectRef],
    exceptions: Vec<PyObjectRef>,
) -> PyObjectRef {
    derive_group(typ, &group_message(args), exceptions)
}

fn derive_group(typ: &str, message: &str, exceptions: Vec<PyObjectRef>) -> PyObjectRef {
    let typ = if typ == "BaseExceptionGroup" && all_are_exception_class(&exceptions) {
        "ExceptionGroup".to_string()
    } else {
        typ.to_string()
    };
    // CPython's `args` for a group is (msg, [exceptions-as-LIST]) — the
    // list is what makes `repr(eg)` read `ExceptionGroup('m', [ValueError()])`.
    let args = vec![py_str(message), py_list(exceptions.clone())];
    PyObjectRef::new(PyObject::ExceptionGroup {
        typ,
        args,
        exceptions,
    })
}

/// `eg.split(condition)` / `eg.subgroup(condition)` — recurses into nested
/// groups (PEP 654: a nested group is matched only if ALL of its children
/// match, and the split keeps it as a subgroup), never flattens.
fn split_group(
    typ: &str,
    message: &str,
    exceptions: Vec<PyObjectRef>,
    condition: &PyObjectRef,
) -> PyResult<(Option<PyObjectRef>, Option<PyObjectRef>)> {
    split_group_at(typ, message, exceptions, condition, 0)
}

/// Nesting depth at which a split gives up, mirroring CPython's C-level
/// recursion protection: a chain of 150k nested groups (what
/// `test.support.exceeds_recursion_limit()` builds) must raise
/// `RecursionError` — not smash the native stack.
const SPLIT_MAX_DEPTH: usize = 1000;

fn split_group_at(
    typ: &str,
    message: &str,
    exceptions: Vec<PyObjectRef>,
    condition: &PyObjectRef,
    depth: usize,
) -> PyResult<(Option<PyObjectRef>, Option<PyObjectRef>)> {
    if depth > SPLIT_MAX_DEPTH {
        return Err(PyError::recursion_error("maximum recursion depth exceeded"));
    }
    // Validate the condition the way CPython does before touching children:
    // an exception type, a tuple of them, or a non-class callable.
    let is_type_like = match &*condition.borrow() {
        PyObject::Type { .. } => true,
        PyObject::BuiltinFunction { .. } => true,
        PyObject::Tuple(items) | PyObject::List(items) => items.iter().all(|i| {
            matches!(
                &*i.borrow(),
                PyObject::Type { .. } | PyObject::BuiltinFunction { .. }
            )
        }),
        _ => false,
    };
    // The predicate stays the CONDITION itself (not its `__call__`): a user
    // class instance needs the full `__call__(self, exc)` dispatch, and
    // handing the raw `__call__` function over loses the `self` binding.
    let predicate = if is_type_like {
        None
    } else {
        let callable = match &*condition.borrow() {
            PyObject::Instance { typ, .. } => {
                crate::object::lookup_dunder_via_mro(typ, "__call__").is_some()
            }
            PyObject::Function { .. }
            | PyObject::BuiltinMethod { .. }
            | PyObject::BoundMethod { .. }
            | PyObject::Closure(_)
            | PyObject::BuiltinFunction { .. } => true,
            _ => false,
        };
        if callable {
            Some(condition.clone())
        } else {
            return Err(PyError::type_error(
                "expected an exception type, a tuple of exception types, or a callable (other than a class)",
            ));
        }
    };
    let mut matched: Vec<PyObjectRef> = Vec::new();
    let mut rest: Vec<PyObjectRef> = Vec::new();
    for child in exceptions {
        let child_parts = match &*child.borrow() {
            PyObject::ExceptionGroup {
                typ,
                args,
                exceptions,
            } => Some((typ.clone(), args.clone(), exceptions.clone())),
            _ => None,
        };
        let (child_typ, child_args, child_exceptions) = match child_parts {
            Some(parts) => parts,
            None => {
                let name = {
                    let borrowed = child.borrow();
                    match &*borrowed {
                        PyObject::Exception { typ, .. } => Some(typ.clone()),
                        _ => None,
                    }
                };
                let name = match name {
                    Some(name) => name,
                    None => {
                        rest.push(child);
                        continue;
                    }
                };
                let hit = match &predicate {
                    Some(pred) => {
                        // A user-defined predicate needs the full call path
                        // (a plain function/callable object), not the
                        // BuiltinFunction-only low-level helper.
                        crate::object::call_function_disposable(pred, vec![child.clone()], vec![])?
                            .truthy()
                    }
                    None => crate::vm::util::exc_type_matches(condition, &name)?,
                };
                if hit {
                    matched.push(child);
                } else {
                    rest.push(child);
                }
                continue;
            }
        };
        let (m, r) = split_group_at(
            &child_typ,
            &group_message(&child_args),
            child_exceptions,
            condition,
            depth + 1,
        )?;
        if let Some(m) = m {
            matched.push(m);
        }
        if let Some(r) = r {
            rest.push(r);
        }
    }
    let matched = if matched.is_empty() {
        None
    } else {
        Some(derive_group(&typ, message, matched))
    };
    let rest = if rest.is_empty() {
        None
    } else {
        Some(derive_group(&typ, message, rest))
    };
    Ok((matched, rest))
}

pub(crate) fn get(o: &PyObject, name: &str) -> PyResult<PyObjectRef> {
    match o {
        PyObject::ExceptionGroup {
            typ,
            args,
            exceptions,
        } => {
            // Owned copies: the PEP 654 methods below hand capturing
            // closures to the VM, which outlive this `&PyObject` borrow.
            let (typ, args, exceptions) = (typ.clone(), args.clone(), exceptions.clone());
            match name {
                "__name__" => Ok(py_str(&typ)),
                "args" => Ok(py_tuple(args.clone())),
                "__str__" => {
                    // CPython: "msg (N sub-exception[s])", or "msg ()" when empty
                    let message = group_message(&args);
                    let n = exceptions.len();
                    let suffix = match n {
                        0 => "()".to_string(),
                        1 => "(1 sub-exception)".to_string(),
                        _ => format!("({} sub-exceptions)", n),
                    };
                    Ok(py_str(&format!("{} {}", message, suffix)))
                }
                "__repr__" => {
                    let message = group_message(&args);
                    let kids: Vec<String> = exceptions.iter().map(|e| e.repr()).collect();
                    Ok(py_str(&format!(
                        "{}({}, [{}])",
                        typ,
                        py_str(&message).repr(),
                        kids.join(", ")
                    )))
                }
                "message" => Ok(args.first().cloned().unwrap_or_else(|| py_str(""))),
                "exceptions" => Ok(py_tuple(exceptions.clone())),
                // PEP 654 API: split/subgroup/derive/__class_getitem__/__reduce__.
                "split" => Ok(PyObjectRef::new(PyObject::Closure(Rc::new(
                    move |call_args: &[PyObjectRef]| {
                        let condition = call_args.first().cloned().unwrap_or_else(py_none);
                        let (matched, rest) = split_group(
                            &typ,
                            &group_message(&args),
                            exceptions.clone(),
                            &condition,
                        )?;
                        Ok(py_tuple(vec![
                            matched.unwrap_or_else(py_none),
                            rest.unwrap_or_else(py_none),
                        ]))
                    },
                )))),
                "subgroup" => Ok(PyObjectRef::new(PyObject::Closure(Rc::new(
                    move |call_args: &[PyObjectRef]| {
                        let condition = call_args.first().cloned().unwrap_or_else(py_none);
                        let (matched, _) = split_group(
                            &typ,
                            &group_message(&args),
                            exceptions.clone(),
                            &condition,
                        )?;
                        Ok(matched.unwrap_or_else(py_none))
                    },
                )))),
                "derive" => Ok(PyObjectRef::new(PyObject::Closure(Rc::new(
                    move |call_args: &[PyObjectRef]| match call_args.first() {
                        Some(new_excs) => {
                            let list = match &*new_excs.borrow() {
                                PyObject::Tuple(items) | PyObject::List(items) => items.clone(),
                                other => {
                                    return Err(PyError::type_error(format!(
                                        "exceptions must be a list, got {}",
                                        other.type_name()
                                    )))
                                }
                            };
                            Ok(derive_group(&typ, &group_message(&args), list))
                        }
                        None => Ok(derive_group(&typ, &group_message(&args), vec![])),
                    },
                )))),
                "__class_getitem__" => Ok(PyObjectRef::new(PyObject::Closure(Rc::new(
                    move |call_args: &[PyObjectRef]| {
                        let origin = PyObjectRef::new(PyObject::BuiltinFunction {
                            name: typ.clone(),
                            func: match typ.as_str() {
                                "ExceptionGroup" => builtin_make_exception_exceptiongroup,
                                _ => builtin_make_exception_baseexceptiongroup,
                            },
                        });
                        Ok(crate::modules::make_generic_alias(
                            origin,
                            call_args.to_vec(),
                        ))
                    },
                )))),
                "__reduce__" => Ok(PyObjectRef::new(PyObject::Closure(Rc::new(
                    move |_call_args: &[PyObjectRef]| {
                        let ctor = PyObjectRef::new(PyObject::BuiltinFunction {
                            name: typ.clone(),
                            func: match typ.as_str() {
                                "ExceptionGroup" => builtin_make_exception_exceptiongroup,
                                _ => builtin_make_exception_baseexceptiongroup,
                            },
                        });
                        let state =
                            vec![py_str(&group_message(&args)), py_tuple(exceptions.clone())];
                        Ok(py_tuple(vec![ctor, py_tuple(state)]))
                    },
                )))),
                // `__traceback__` needs the owning `PyObjectRef` (the chain is
                // keyed by object identity, see `object::eg_traceback`), so it is
                // answered by the ref-level fast paths in `vm/op_attr.rs`
                // (LOAD_ATTR) and the `getattr` builtin instead of here.
                "__cause__" | "__context__" | "__traceback__" => Ok(py_none()),
                "__suppress_context__" => Ok(py_bool(false)),
                "__notes__" => Ok(py_list(vec![])),
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
                _ => Err(PyError::attribute_error(format!(
                    "'{}' object has no attribute '{}'",
                    typ, name
                ))),
            }
        }
        _ => unreachable!(
            "{} called with non-{} object",
            stringify!(get),
            o.type_name()
        ),
    }
}
