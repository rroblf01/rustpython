// Split out of the former monolithic object/builtins.rs — this file holds
// the `print` builtin and its helper (`print_with_vm`/`call_method_rebound`).
use super::*;

// Fallback only — real dispatch goes through `print_with_vm` via a
// `fn_addr_eq` special-case in `vm.rs`'s `call_function` (see
// `print_with_vm`'s own doc comment for why this needs the live VM).
pub fn builtin_print(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    with_vm_mut(|vm| print_with_vm(vm, args, &[]))?
}

/// The real `print()` implementation — needs a live `&mut VirtualMachine` to
/// look up the CURRENT value of `sys.stdout` (not a cached reference) so
/// `contextlib.redirect_stdout`/`unittest.mock.patch('sys.stdout', ...)`-style
/// substitution actually takes effect, and to call arbitrary objects'
/// `write`/`flush` methods (a plain `io::stdout()` `println!()`, which is
/// what this used to be, can reach neither). Previously this also silently
/// ignored `sep`/`end`/`file`/`flush` keyword arguments entirely — they were
/// packed into a trailing dict (this project's established kwargs-passing
/// convention for plain `BuiltinFunction`s) and then that dict got PRINTED
/// AS A POSITIONAL ARGUMENT, since the old code just joined every element of
/// `args` unconditionally. Confirmed via the simplest possible repro:
/// `print("x", end="")` printed `x {'end': ''}` instead of `x` with no
/// trailing newline. Given how extremely common `sep=`/`end=`/`file=` and
/// stdout-capturing test patterns both are in real Python code, this was one
/// of the most broadly-impactful gaps found this session.
pub(crate) fn print_with_vm(
    vm: &mut crate::vm::VirtualMachine,
    args: &[PyObjectRef],
    keywords: &[(String, PyObjectRef)],
) -> PyResult<PyObjectRef> {
    let mut sep = " ".to_string();
    let mut end = "\n".to_string();
    let mut file: Option<PyObjectRef> = None;
    let mut flush = false;
    for (k, v) in keywords {
        match k.as_str() {
            "sep" => {
                if !matches!(&*v.borrow(), PyObject::None) {
                    // `print(..., sep=3)` must raise TypeError (real CPython:
                    // "sep must be None or a string"), not stringify — a
                    // plain `.str()` silently coerced any value.
                    if !matches!(&*v.borrow(), PyObject::Str(_)) {
                        return Err(PyError::type_error(format!(
                            "sep must be None or a string, not {}",
                            v.borrow().type_name()
                        )));
                    }
                    sep = v.str();
                }
            }
            "end" => {
                if !matches!(&*v.borrow(), PyObject::None) {
                    if !matches!(&*v.borrow(), PyObject::Str(_)) {
                        return Err(PyError::type_error(format!(
                            "end must be None or a string, not {}",
                            v.borrow().type_name()
                        )));
                    }
                    end = v.str();
                }
            }
            "file" => {
                if !matches!(&*v.borrow(), PyObject::None) {
                    file = Some(v.clone());
                }
            }
            "flush" => {
                flush = v.truthy();
            }
            _ => {}
        }
    }

    let strings: Vec<String> = args.iter().map(|a| a.str()).collect();
    let mut output = strings.join(&sep);
    output.push_str(&end);

    let target = match file {
        Some(f) => f,
        None => vm
            .modules
            .get("sys")
            .and_then(|m| {
                if let PyObject::Module { dict, .. } = &*m.borrow() {
                    dict.get_str("stdout").cloned()
                } else {
                    None
                }
            })
            .ok_or_else(|| PyError::runtime_error("lost sys.stdout"))?,
    };

    call_method_rebound(vm, &target, "write", vec![py_str(&output)])
        .map_err(|_| PyError::attribute_error("'file' object has no attribute 'write'"))?;

    if flush {
        // A raising `flush` must PROPAGATE (real CPython: `print(x,
        // file=f, flush=True)` surfaces f.flush()'s exception —
        // test_print.py::test_print_flush asserts RuntimeError passes
        // through). Was `let _ =` swallowing it.
        call_method_rebound(vm, &target, "flush", vec![])?;
    }

    Ok(py_none())
}

/// Calls `target.<name>(call_args...)`, rebinding a native `BuiltinMethod`'s
/// `self_obj` to `target` directly (ONE prepended self, matching how
/// `LOAD_ATTR` itself rebinds container methods like `File`/`List`/`Dict`'s
/// `write`/`append`/etc. for ordinary dot-call syntax) — NOT
/// `call_bound_method`'s convention, which prepends BOTH the method's own
/// (placeholder) `self_obj` AND an explicit second one, meant for dunder
/// methods that are written expecting that double-self shape. Using
/// `call_bound_method` here initially caused `File::write`'s own `args[0]`
/// check to see the leftover placeholder instead of the real file, raising
/// "write on non-file" — confirmed by testing plain `f.write(x)` (which
/// goes through `LOAD_ATTR`'s rebind-in-place logic, not `call_bound_method`)
/// working correctly on the exact same object.
pub(crate) fn call_method_rebound(
    vm: &mut crate::vm::VirtualMachine,
    target: &PyObjectRef,
    name: &str,
    call_args: Vec<PyObjectRef>,
) -> PyResult<PyObjectRef> {
    let method = target.borrow().get_attribute(name)?;
    let bound = match &*method.borrow() {
        // `get_attribute` may have already rebound `self_obj` to something
        // OTHER than `target` (e.g. a native-backed instance's method
        // resolved against its native backing, not the Instance wrapper —
        // see instance.rs's NATIVE_BACKING_KEY delegation). Only fall back
        // to `target` when `self_obj` is still the unbound placeholder
        // (`PyObject::None`); otherwise keep what `get_attribute` resolved,
        // or e.g. `OrderedDict.keys()` ends up called with the wrapper
        // Instance instead of its backing Dict ("keys on non-dict").
        PyObject::BuiltinMethod {
            func, name: mname, self_obj,
        } => PyObjectRef::imm(PyObject::BuiltinMethod {
            name: mname.clone(),
            func: *func,
            self_obj: if matches!(&*self_obj.borrow(), PyObject::None) {
                target.clone()
            } else {
                self_obj.clone()
            },
        }),
        // A user-defined method (raw `Function` from the type dict — the
        // ObjectAccess `get_attribute` trait doesn't auto-bind, unlike
        // LOAD_ATTR) must be wrapped in a BoundMethod so `self` is prepended.
        // Without this, `print(..., file=custom_filelike)` calling the
        // object's `write` invoked it with one argument missing (its own
        // `self`), raising a TypeError mapped to a bogus "'file' object has
        // no attribute 'write'" (test_print.py::test_print_flush).
        PyObject::Function(_) => PyObjectRef::new(PyObject::BoundMethod {
            func: method.clone(),
            self_obj: target.clone(),
        }),
        // A native method stored as a plain `BuiltinFunction` in a type's
        // dict (e.g. `OrderedDict.__getitem__`) also isn't auto-bound by
        // the `get_attribute` trait — wrap it the same way LOAD_ATTR's own
        // auto-bind does (op_attr.rs), except for the same two excluded
        // shapes: a builtin exception "class" reference and `open`, neither
        // of which are real bound methods.
        PyObject::BuiltinFunction { name: n, func }
            if !(crate::object::is_builtin_exception_class_name(n)
                || std::ptr::fn_addr_eq(*func, crate::object::builtin_open as crate::object::BuiltinFunc)) =>
        {
            PyObjectRef::new(PyObject::BoundMethod {
                func: method.clone(),
                self_obj: target.clone(),
            })
        }
        _ => method.clone(),
    };
    vm.call_function(bound, call_args, vec![])
}
