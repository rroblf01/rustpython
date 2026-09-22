//! PEP 750 template strings: `Template` and `Interpolation` heap types.
//!
//! Ground truth: CPython `Objects/templateobject.c` +
//! `Objects/interpolationobject.c` (native types), surfaced as
//! `string.templatelib.Template` / `.Interpolation`.
//!
//! These are *heap* types (`PyObject::Type` + `PyObject::Instance`,
//! same machinery as user classes and `pathlib.Path`) rather than new
//! `PyObject` variants — instances only ever carry tuples/strings/None,
//! so no new match sites are needed anywhere: attribute access,
//! `isinstance`, `type()`, `match` class patterns, `repr`, `+` and
//! `iter()` all flow through the existing Instance paths.

use crate::object::*;
use std::collections::HashMap;

thread_local! {
    static TEMPLATE_TYPE: std::cell::RefCell<Option<PyObjectRef>> =
        std::cell::RefCell::new(None);
    static INTERPOLATION_TYPE: std::cell::RefCell<Option<PyObjectRef>> =
        std::cell::RefCell::new(None);
}

fn builtin_fn(name: &str, func: BuiltinFunc) -> PyObjectRef {
    PyObjectRef::new(PyObject::BuiltinFunction {
        name: name.to_string(),
        func,
    })
}

fn make_type(name: &str, dict: HashMap<String, PyObjectRef>) -> PyObjectRef {
    let mut type_dict = str_map_to_typedict(dict);
    type_dict.insert_str("__module__", py_str("string.templatelib"));
    let typ = PyObjectRef::new(PyObject::Type {
        name: name.to_string(),
        dict: Box::new(type_dict),
        bases: vec![],
        mro: vec![],
    });
    if let PyObject::Type { mro, .. } = &mut *typ.borrow_mut() {
        *mro = vec![typ.clone()];
    }
    typ
}

/// The cached `Template` heap type, creating it (and `Interpolation`'s)
/// on first use.
pub(crate) fn template_type() -> PyResult<PyObjectRef> {
    if let Some(t) = TEMPLATE_TYPE.with(|c| c.borrow().clone()) {
        return Ok(t);
    }
    ensure_template_types()
}

/// The cached `Interpolation` heap type.
pub(crate) fn interpolation_type() -> PyResult<PyObjectRef> {
    if let Some(t) = INTERPOLATION_TYPE.with(|c| c.borrow().clone()) {
        return Ok(t);
    }
    ensure_template_types().and_then(|_| {
        INTERPOLATION_TYPE.with(|c| {
            c.borrow().clone().ok_or_else(|| {
                PyError::runtime_error("Interpolation type not initialized".to_string())
            })
        })
    })
}

fn ensure_template_types() -> PyResult<PyObjectRef> {
    if let Some(t) = TEMPLATE_TYPE.with(|c| c.borrow().clone()) {
        return Ok(t);
    }

    // --- Interpolation type ---
    let mut interp_dict = HashMap::new();
    interp_dict.insert_str(
        "__match_args__",
        py_tuple(vec![
            py_str("value"),
            py_str("expression"),
            py_str("conversion"),
            py_str("format_spec"),
        ]),
    );
    interp_dict.insert_str("__repr__", builtin_fn("__repr__", interpolation_repr));
    interp_dict.insert_str("__new__", builtin_fn("__new__", template_new_wrapper));
    let interpolation_type = make_type("Interpolation", interp_dict);

    // --- Template type ---
    let mut template_dict = HashMap::new();
    template_dict.insert_str("__repr__", builtin_fn("__repr__", template_repr));
    template_dict.insert_str("__iter__", builtin_fn("__iter__", template_iter));
    template_dict.insert_str("__add__", builtin_fn("__add__", template_add));
    template_dict.insert_str("__radd__", builtin_fn("__radd__", template_radd));
    template_dict.insert_str("__new__", builtin_fn("__new__", template_new_wrapper));
    let values_getter = builtin_fn("values", |args| {
        if args.is_empty() {
            return Err(PyError::type_error(
                "values getter missing argument".to_string(),
            ));
        }
        template_values(&args[0])
    });
    template_dict.insert_str(
        "values",
        PyObjectRef::new(PyObject::Property(Box::new(PropertyData {
            getter: Some(values_getter),
            setter: None,
            deleter: None,
            doc: None,
        }))),
    );
    let template_type = make_type("Template", template_dict);

    TEMPLATE_TYPE.with(|c| *c.borrow_mut() = Some(template_type.clone()));
    INTERPOLATION_TYPE.with(|c| *c.borrow_mut() = Some(interpolation_type));
    Ok(template_type)
}

fn template_parts(obj: &PyObjectRef) -> PyResult<(Vec<PyObjectRef>, Vec<PyObjectRef>)> {
    let template_typ = template_type()?;
    match &*obj.borrow() {
        PyObject::Instance { typ, dict } if typ.is(&template_typ) => {
            let strings = match dict.get("strings") {
                Some(r) => match &*r.borrow() {
                    PyObject::Tuple(v) => v.clone(),
                    _ => {
                        return Err(PyError::runtime_error(
                            "Template corrupted: strings is not a tuple".to_string(),
                        ))
                    }
                },
                None => {
                    return Err(PyError::runtime_error(
                        "Template corrupted: missing strings".to_string(),
                    ))
                }
            };
            let interpolations = match dict.get("interpolations") {
                Some(r) => match &*r.borrow() {
                    PyObject::Tuple(v) => v.clone(),
                    _ => {
                        return Err(PyError::runtime_error(
                            "Template corrupted: interpolations is not a tuple".to_string(),
                        ))
                    }
                },
                None => {
                    return Err(PyError::runtime_error(
                        "Template corrupted: missing interpolations".to_string(),
                    ))
                }
            };
            Ok((strings, interpolations))
        }
        _ => Err(PyError::type_error(
            "expected Template instance".to_string(),
        )),
    }
}

/// Build a `Template` from already-padded `strings` and `interpolations`
/// tuples (mirrors CPython `_PyTemplate_Build`).
pub(crate) fn build_template(
    strings: Vec<PyObjectRef>,
    interpolations: Vec<PyObjectRef>,
) -> PyResult<PyObjectRef> {
    let typ = template_type()?;
    let mut dict = AttrMap::new();
    dict.insert_str("strings", py_tuple(strings));
    dict.insert_str("interpolations", py_tuple(interpolations));
    Ok(PyObjectRef::new(PyObject::Instance { typ, dict }))
}

/// Build an `Interpolation` (mirrors CPython `_PyInterpolation_Build`:
/// conversion is `None` or a one-character `'s'`/`'r'`/`'a'` string).
pub(crate) fn build_interpolation(
    value: PyObjectRef,
    expression: String,
    conversion: Option<char>,
    format_spec: PyObjectRef,
) -> PyResult<PyObjectRef> {
    let typ = interpolation_type()?;
    let mut dict = AttrMap::new();
    dict.insert_str("value", value);
    dict.insert_str("expression", py_str(&expression));
    dict.insert_str(
        "conversion",
        match conversion {
            Some(c) => py_str(&c.to_string()),
            None => py_none(),
        },
    );
    dict.insert_str("format_spec", format_spec);
    Ok(PyObjectRef::new(PyObject::Instance { typ, dict }))
}

fn template_repr(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error(
            "expected Template instance".to_string(),
        ));
    }
    let (strings, interpolations) = template_parts(&args[0])?;
    let type_name = match &*args[0].borrow() {
        PyObject::Instance { typ, .. } => match &*typ.borrow() {
            PyObject::Type { name, .. } => name.clone(),
            _ => "Template".to_string(),
        },
        _ => "Template".to_string(),
    };
    Ok(py_str(&format!(
        "{}(strings={}, interpolations={})",
        type_name,
        py_tuple(strings).borrow().repr(),
        py_tuple(interpolations).borrow().repr(),
    )))
}

fn interpolation_repr(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error(
            "expected Interpolation instance".to_string(),
        ));
    }
    let interp_typ = interpolation_type()?;
    let (value, expression, conversion, format_spec) = match &*args[0].borrow() {
        PyObject::Instance { typ, dict } if typ.is(&interp_typ) => (
            dict.get("value").cloned(),
            dict.get("expression").cloned(),
            dict.get("conversion").cloned(),
            dict.get("format_spec").cloned(),
        ),
        _ => {
            return Err(PyError::type_error(
                "expected Interpolation instance".to_string(),
            ))
        }
    };
    let (value, expression, conversion, format_spec) =
        match (value, expression, conversion, format_spec) {
            (Some(v), Some(e), Some(c), Some(f)) => (v, e, c, f),
            _ => {
                return Err(PyError::runtime_error(
                    "Interpolation corrupted: missing field".to_string(),
                ))
            }
        };
    let type_name = match &*args[0].borrow() {
        PyObject::Instance { typ, .. } => match &*typ.borrow() {
            PyObject::Type { name, .. } => name.clone(),
            _ => "Interpolation".to_string(),
        },
        _ => "Interpolation".to_string(),
    };
    Ok(py_str(&format!(
        "{}({}, {}, {}, {})",
        type_name,
        value.borrow().repr(),
        expression.borrow().repr(),
        conversion.borrow().repr(),
        format_spec.borrow().repr(),
    )))
}

/// `iter(t)` — interleaves strings and interpolations, skipping empty
/// strings (mirrors CPython `templateiter_next`).
fn template_iter(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error(
            "expected Template instance".to_string(),
        ));
    }
    let (strings, interpolations) = template_parts(&args[0])?;
    let mut items: Vec<PyObjectRef> = vec![];
    for (i, s) in strings.iter().enumerate() {
        let is_empty = matches!(&*s.borrow(), PyObject::Str(st) if st.as_str().is_empty());
        if is_empty {
            if let Some(interp) = interpolations.get(i) {
                items.push(interp.clone());
            }
        } else {
            items.push(s.clone());
            if let Some(interp) = interpolations.get(i) {
                items.push(interp.clone());
            }
        }
    }
    crate::object::builtin_iter(&[py_list(items)])
}

/// `t1 + t2` — merges adjacent boundary strings (mirrors CPython
/// `template_concat_templates`); anything else is a TypeError.
fn template_add(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 2 {
        return Err(PyError::type_error(
            "expected Template instance".to_string(),
        ));
    }
    let template_typ = template_type()?;
    let other_is_template =
        matches!(&*args[1].borrow(), PyObject::Instance { typ, .. } if typ.is(&template_typ));
    if !other_is_template {
        let other_name = args[1].borrow().type_name();
        return Err(PyError::type_error(format!(
            "can only concatenate string.templatelib.Template (not \"{}\") to string.templatelib.Template",
            other_name
        )));
    }
    let (lstrings, linterps) = template_parts(&args[0])?;
    let (rstrings, rinterps) = template_parts(&args[1])?;
    let mut strings: Vec<PyObjectRef> = vec![];
    for s in lstrings.iter().take(lstrings.len().saturating_sub(1)) {
        strings.push(s.clone());
    }
    let boundary = format!(
        "{}{}",
        lstrings.last().map(|s| s.str()).unwrap_or_default(),
        rstrings.first().map(|s| s.str()).unwrap_or_default()
    );
    strings.push(py_str(&boundary));
    for s in rstrings.iter().skip(1) {
        strings.push(s.clone());
    }
    let mut interpolations = linterps;
    interpolations.extend(rinterps);
    build_template(strings, interpolations)
}

/// `"s" + t` — `str` has no `__add__` for Templates, so this reflected
/// arm produces CPython's exact message.
fn template_radd(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 2 {
        return Err(PyError::type_error(
            "expected Template instance".to_string(),
        ));
    }
    Err(PyError::type_error(
        "can only concatenate str (not \"string.templatelib.Template\") to str".to_string(),
    ))
}

/// The read-only `.values` property: tuple of interpolation values
/// (mirrors CPython `template_values_get`).
fn template_values(obj: &PyObjectRef) -> PyResult<PyObjectRef> {
    let (_, interpolations) = template_parts(obj)?;
    let interp_typ = interpolation_type()?;
    let mut values = vec![];
    for interp in &interpolations {
        match &*interp.borrow() {
            PyObject::Instance { typ, dict } if typ.is(&interp_typ) => match dict.get("value") {
                Some(v) => values.push(v.clone()),
                None => {
                    return Err(PyError::runtime_error(
                        "Interpolation corrupted: missing value".to_string(),
                    ))
                }
            },
            _ => {
                return Err(PyError::type_error(
                    "expected Interpolation instance".to_string(),
                ))
            }
        }
    }
    Ok(py_tuple(values))
}

/// Which template family `callable` belongs to: the `Template` /
/// `Interpolation` heap type itself or a subclass of one (matched
/// through the MRO, mirroring how CPython inherits `__new__`).
fn template_kind_of(callable: &PyObjectRef) -> PyResult<Option<bool>> {
    // Returns Ok(Some(true)) for Template, Ok(Some(false)) for
    // Interpolation, Ok(None) for anything else.
    let template_typ = template_type()?;
    let interpolation_typ = interpolation_type()?;
    let chain: Vec<PyObjectRef> = match &*callable.borrow() {
        PyObject::Type { mro, .. } => {
            let mut v = vec![callable.clone()];
            v.extend(mro.clone());
            v
        }
        _ => return Ok(None),
    };
    for entry in &chain {
        if entry.is(&template_typ) {
            return Ok(Some(true));
        }
        if entry.is(&interpolation_typ) {
            return Ok(Some(false));
        }
    }
    Ok(None)
}

/// `Template.__new__` / `Interpolation.__new__` as seen through generic
/// dispatch (direct `Template.__new__(cls, ...)` calls): `args[0]` is the
/// class, and keyword arguments arrive packed in a trailing dict per the
/// plain-builtin convention. Unpacks both and delegates to `template_new`.
fn template_new_wrapper(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error(
            "__new__() missing required argument 'cls'".to_string(),
        ));
    }
    let cls = &args[0];
    let mut rest = &args[1..];
    let mut keywords: Vec<(String, PyObjectRef)> = vec![];
    if let Some((last, prefix)) = rest.split_last() {
        if matches!(&*last.borrow(), PyObject::Dict(_)) {
            if let PyObject::Dict(d) = &*last.borrow() {
                for (k, v) in d.items() {
                    keywords.push((k.str(), v.clone()));
                }
            }
            rest = prefix;
        }
    }
    match template_new(cls, rest, &keywords)? {
        Some(obj) => Ok(obj),
        None => Err(PyError::type_error(
            "__new__() called on a non-template class".to_string(),
        )),
    }
}

/// Direct construction entry point used by the `handle_type_call`
/// fast path (which sees real `args` + `keywords`, unlike the packed
/// trailing-dict convention of plain builtins): `Template(*args)` /
/// `Interpolation(value, expression="", conversion=None,
/// format_spec="")`, mirroring CPython `template_new` /
/// `interpolation_new_impl` including exact `TypeError` texts.
/// The instance's type is `callable` itself (so subclasses work).
pub(crate) fn template_new(
    callable: &PyObjectRef,
    args: &[PyObjectRef],
    keywords: &[(String, PyObjectRef)],
) -> PyResult<Option<PyObjectRef>> {
    let kind = template_kind_of(callable)?;
    let is_template = match kind {
        Some(k) => k,
        None => return Ok(None),
    };
    if is_template {
        if !keywords.is_empty() {
            return Err(PyError::type_error(
                "Template.__new__ only accepts *args arguments".to_string(),
            ));
        }
        // Partition args into strings (merging adjacent ones) and
        // interpolations, padding empty strings exactly like the
        // compiler-emitted BUILD_TEMPLATE sequence does.
        let interp_typ = interpolation_type()?;
        let mut strings: Vec<String> = vec![];
        let mut interpolations: Vec<PyObjectRef> = vec![];
        let mut last_was_str = false;
        for item in args {
            let b = item.borrow();
            match &*b {
                PyObject::Str(s) => {
                    if last_was_str {
                        let last = strings.pop().unwrap_or_default();
                        strings.push(format!("{}{}", last, s.as_str()));
                    } else {
                        strings.push(s.as_str().to_string());
                    }
                    last_was_str = true;
                }
                PyObject::Instance { typ, .. } if typ.is(&interp_typ) => {
                    if !last_was_str {
                        strings.push(String::new());
                    }
                    drop(b);
                    interpolations.push(item.clone());
                    last_was_str = false;
                }
                _ => {
                    let tname = b.type_name();
                    drop(b);
                    return Err(PyError::type_error(format!(
                        "Template.__new__ *args need to be of type 'str' or 'Interpolation', got {}",
                        tname
                    )));
                }
            }
        }
        if !last_was_str {
            strings.push(String::new());
        }
        let mut dict = AttrMap::new();
        dict.insert_str(
            "strings",
            py_tuple(strings.into_iter().map(|s| py_str(&s)).collect()),
        );
        dict.insert_str("interpolations", py_tuple(interpolations));
        Ok(Some(PyObjectRef::new(PyObject::Instance {
            typ: callable.clone(),
            dict,
        })))
    } else {
        // Interpolation(value, expression="", conversion=None, format_spec="")
        const FIELDS: [&str; 4] = ["value", "expression", "conversion", "format_spec"];
        let mut values: [Option<PyObjectRef>; 4] = [None, None, None, None];
        if args.len() > 4 {
            return Err(PyError::type_error(format!(
                "Interpolation() takes from 1 to 4 positional arguments but {} were given",
                args.len()
            )));
        }
        for (i, v) in args.iter().enumerate() {
            values[i] = Some(v.clone());
        }
        for (k, v) in keywords {
            match FIELDS.iter().position(|f| *f == k) {
                Some(i) => {
                    if values[i].is_some() {
                        return Err(PyError::type_error(format!(
                            "Interpolation() got multiple values for argument '{}'",
                            k
                        )));
                    }
                    values[i] = Some(v.clone());
                }
                None => {
                    return Err(PyError::type_error(format!(
                        "Interpolation() got an unexpected keyword argument '{}'",
                        k
                    )));
                }
            }
        }
        let value = values[0].clone().ok_or_else(|| {
            PyError::type_error(
                "Interpolation() missing 1 required positional argument: 'value'".to_string(),
            )
        })?;
        let expression = match &values[1] {
            None => py_str(""),
            Some(e) => match &*e.borrow() {
                PyObject::Str(s) => py_str(s.as_str()),
                _ => {
                    let tname = e.borrow().type_name();
                    return Err(PyError::type_error(format!(
                        "Interpolation() argument 'expression' must be str, not {}",
                        tname
                    )));
                }
            },
        };
        let conversion =
            match &values[2] {
                None => py_none(),
                Some(c) => match &*c.borrow() {
                    PyObject::None => py_none(),
                    PyObject::Str(s) => match s.as_str() {
                        "s" | "r" | "a" => c.clone(),
                        _ => return Err(PyError::type_error(
                            "Interpolation() argument 'conversion' must be one of 's', 'a' or 'r'"
                                .to_string(),
                        )),
                    },
                    _ => {
                        let tname = c.borrow().type_name();
                        return Err(PyError::type_error(format!(
                            "Interpolation() argument 'conversion' must be str, not {}",
                            tname
                        )));
                    }
                },
            };
        let format_spec = match &values[3] {
            None => py_str(""),
            Some(f) => match &*f.borrow() {
                PyObject::Str(_) => f.clone(),
                _ => {
                    let tname = f.borrow().type_name();
                    return Err(PyError::type_error(format!(
                        "Interpolation() argument 'format_spec' must be str, not {}",
                        tname
                    )));
                }
            },
        };
        let mut dict = AttrMap::new();
        dict.insert_str("value", value);
        dict.insert_str("expression", expression);
        dict.insert_str("conversion", conversion);
        dict.insert_str("format_spec", format_spec);
        Ok(Some(PyObjectRef::new(PyObject::Instance {
            typ: callable.clone(),
            dict,
        })))
    }
}
