// Auto-extracted from src/object/attrs/mod.rs lines 4182-5013
use crate::object::*;
use super::*;
use crate::interner;

pub(crate) fn get(o: &PyObject, name: &str) -> PyResult<PyObjectRef> {
    match o {
            PyObject::Str(_s) => {
                match name {
                    "__mul__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__mul__".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error("__mul__() missing argument"));
                            }
                            dunder_repeat(
                                &args[args.len().saturating_sub(2)],
                                &args[args.len().saturating_sub(1)],
                            )
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__rmul__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__rmul__".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error("__rmul__() missing argument"));
                            }
                            dunder_repeat(
                                &args[args.len().saturating_sub(2)],
                                &args[args.len().saturating_sub(1)],
                            )
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    // Gettable `__hash__` — needed so `super().__hash__()`
                    // works for a `class K(str): def __hash__(self): ...
                    // return super().__hash__()` override (the `super()`
                    // proxy's own attribute resolution falls back to the
                    // native backing's `get_attribute`, which previously had
                    // no `__hash__` case at all here — real trigger:
                    // CPython's own `test_baseexception.py::
                    // test_setstate_refcount_no_crash`, gh-97591).
                    "__hash__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__hash__".to_string(),
                        func: |args| Ok(py_int(args[0].hash()? as i64)),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__contains__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__contains__".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "__contains__() takes exactly one argument",
                                ));
                            }
                            py_contains(&args[0], &args[1])
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "format" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "format".to_string(),
                        func: |args| {
                            if args.is_empty() {
                                return Err(PyError::type_error(
                                    "format() takes at least 1 argument",
                                ));
                            }
                            let fmt = args[0].str();
                            // Keyword arguments arrive packed into a trailing
                            // dict (this project's established calling
                            // convention for native methods — see
                            // `call_function`'s `BuiltinMethod` arm in
                            // vm.rs). If the last arg is a Dict, treat it as
                            // the kwargs pack for named fields and exclude
                            // it from positional indexing — previously
                            // named fields (`'{name}'.format(name=...)`)
                            // were entirely unimplemented and silently
                            // printed the field NAME itself instead of its
                            // value (confirmed via CPython's own
                            // `test_listcomps.py`, which builds source code
                            // via `"...{code}...".format(code=code)`).
                            let rest = &args[1..];
                            // A trailing Dict is only the kwargs pack when the
                            // format string actually uses NAMED fields
                            // (`'{name}'`); otherwise a dict passed as an
                            // ordinary positional arg (e.g.
                            // `'{}'.format({b'a': [b'b']})`, real trigger:
                            // test_urlparse's `_SubTest.__str__` formatting
                            // subTest params that are themselves dicts) would
                            // be wrongly eaten as the kwargs pack, leaving
                            // zero positional args.
                            let has_named_fields = {
                                let mut saw_named = false;
                                let mut in_field = false;
                                let mut in_suffix = false;
                                let mut name_part = String::new();
                                for c in fmt.chars() {
                                    if c == '{' {
                                        if !in_field {
                                            in_field = true;
                                            name_part.clear();
                                            in_suffix = false;
                                        }
                                    } else if c == '}' {
                                        if in_field
                                            && !name_part.is_empty()
                                            && !name_part.chars().all(|ch| ch.is_ascii_digit())
                                            && !name_part.starts_with(|ch: char| ch.is_ascii_digit())
                                        {
                                            saw_named = true;
                                        }
                                        in_field = false;
                                    } else if in_field {
                                        // Only the NAME portion (before any
                                        // `!conversion` / `:spec`) determines
                                        // whether the field is named — stop
                                        // collecting once a suffix starts.
                                        if c == '!' || c == ':' {
                                            in_suffix = true;
                                            continue;
                                        }
                                        if !in_suffix && !c.is_whitespace() {
                                            name_part.push(c);
                                        }
                                    }
                                }
                                saw_named
                            };
                            let kwargs_dict: Option<PyObjectRef> = match rest.last() {
                                Some(a)
                                    if has_named_fields
                                        && matches!(&*a.borrow(), PyObject::Dict(_)) =>
                                {
                                    Some(a.clone())
                                }
                                _ => None,
                            };
                            let pos_args: &[PyObjectRef] = if kwargs_dict.is_some() {
                                &rest[..rest.len() - 1]
                            } else {
                                rest
                            };
                            let mut result = String::new();
                            let mut chars = fmt.chars();
                            let mut next_auto = 0usize;
                            let mut used_manual_numbering = false;
                            let mut used_auto_numbering = false;
                            // Resolve nested `{field}` replacements inside a
                            // format spec string (e.g. the `{}` in `{:0{}x}`
                            // takes the next arg as the width). Returns the
                            // spec with each nested field's value substituted.
                            let resolve_nested_spec =
                                |spec: &str,
                                 next_auto: &mut usize,
                                 used_manual_numbering: &mut bool,
                                 used_auto_numbering: &mut bool,
                                 pos_args: &[PyObjectRef],
                                 kwargs_dict: Option<&PyObjectRef>|
                                 -> PyResult<String> {
                                    let mut out = String::new();
                                    let mut sc = spec.chars();
                                    while let Some(c) = sc.next() {
                                        if c == '{' {
                                            let mut inner = String::new();
                                            loop {
                                                match sc.next() {
                                                    Some('}') => break,
                                                    Some(c) => inner.push(c),
                                                    None => {
                                                        return Err(PyError::value_error(
                                                            "unterminated nested format field",
                                                        ))
                                                    }
                                                }
                                            }
                                            let inner = inner.trim();
                                            if inner.is_empty() {
                                                if *used_manual_numbering {
                                                    return Err(PyError::value_error("cannot switch from manual field specification to automatic field numbering"));
                                                }
                                                *used_auto_numbering = true;
                                                let idx = *next_auto;
                                                *next_auto += 1;
                                                match pos_args.get(idx) {
                                                Some(v) => out.push_str(&v.str()),
                                                None => return Err(PyError::index_error("Replacement index out of range for positional args tuple")),
                                            }
                                            } else if let Ok(n) = inner.parse::<usize>() {
                                                if *used_auto_numbering {
                                                    return Err(PyError::value_error("cannot switch from automatic field numbering to manual field specification"));
                                                }
                                                *used_manual_numbering = true;
                                                match pos_args.get(n) {
                                                Some(v) => out.push_str(&v.str()),
                                                None => return Err(PyError::index_error("Replacement index out of range for positional args tuple")),
                                            }
                                            } else {
                                                match kwargs_dict.and_then(|d| {
                                                    if let PyObject::Dict(dd) = &*d.borrow() {
                                                        dd.get(&py_str(inner)).ok().flatten()
                                                    } else {
                                                        None
                                                    }
                                                }) {
                                                    Some(v) => out.push_str(&v.str()),
                                                    None => {
                                                        return Err(PyError::key_error(format!(
                                                            "'{}'",
                                                            inner
                                                        )))
                                                    }
                                                }
                                            }
                                        } else {
                                            out.push(c);
                                        }
                                    }
                                    Ok(out)
                                };
                            while let Some(c) = chars.next() {
                                if c == '{' {
                                    // Check for {{ escape
                                    if chars.as_str().starts_with('{') {
                                        result.push('{');
                                        chars.next();
                                        continue;
                                    }
                                    // Parse field text up to the matching `}`,
                                    // tracking nested braces: `{:0{}x}`'s
                                    // inner `{}` must not close the field.
                                    let mut field = String::new();
                                    let mut depth = 0usize;
                                    loop {
                                        match chars.next() {
                                            Some('}') if depth == 0 => break,
                                            Some('}') => {
                                                depth -= 1;
                                                field.push('}');
                                            }
                                            Some('{') => {
                                                depth += 1;
                                                field.push('{');
                                            }
                                            Some(c) => field.push(c),
                                            None => {
                                                return Err(PyError::value_error(
                                                    "unterminated format field",
                                                ))
                                            }
                                        }
                                    }
                                    // Split off an optional `!conversion` and
                                    // `:spec` suffix — previously not parsed
                                    // at all, so even POSITIONAL fields with
                                    // a spec (`{0:>10}`) printed the raw
                                    // field text instead of applying it.
                                    let (name_part, spec) = match field.find(':') {
                                        Some(idx) => (&field[..idx], &field[idx + 1..]),
                                        None => (field.as_str(), ""),
                                    };
                                    let (name_part, conversion) = match name_part.find('!') {
                                        Some(idx) => {
                                            (&name_part[..idx], Some(&name_part[idx + 1..]))
                                        }
                                        None => (name_part, None),
                                    };
                                    // Resolve the field's value: auto `{}`,
                                    // positional `{0}`, or named `{key}`
                                    // (looked up in the trailing kwargs dict).
                                    let val: PyResult<PyObjectRef> = if name_part.is_empty() {
                                        if used_manual_numbering {
                                            return Err(PyError::value_error("cannot switch from manual field specification to automatic field numbering"));
                                        }
                                        used_auto_numbering = true;
                                        let idx = next_auto;
                                        next_auto += 1;
                                        pos_args.get(idx).cloned()
                                            .ok_or_else(|| PyError::index_error("Replacement index out of range for positional args tuple"))
                                    } else if let Ok(n) = name_part.parse::<usize>() {
                                        if used_auto_numbering {
                                            return Err(PyError::value_error("cannot switch from automatic field numbering to manual field specification"));
                                        }
                                        used_manual_numbering = true;
                                        pos_args.get(n).cloned()
                                            .ok_or_else(|| PyError::index_error("Replacement index out of range for positional args tuple"))
                                    } else {
                                        // Could be "N[...]" (positional index
                                        // with subscript) or a named field.
                                        // CPython handles {0[0]} as arg[0][0],
                                        // while {name} does dict lookup.
                                        if let Some(bracket_pos) = name_part.find('[') {
                                            // Positional with subscript: "{0[0]}"
                                            let idx_str = &name_part[..bracket_pos];
                                            let sub_str = &name_part[bracket_pos+1..name_part.len()-1];
                                            if let Ok(idx) = idx_str.parse::<usize>() {
                                                if let Some(obj) = pos_args.get(idx) {
                                                    // Apply subscript: obj[sub_str]
                                                    if let Ok(sub_idx) = sub_str.parse::<usize>() {
                                                        let sub_obj = match &*obj.borrow() {
                                                            PyObject::Tuple(t) => t.get(sub_idx).cloned(),
                                                            PyObject::List(l) => l.get(sub_idx).cloned(),
                                                            PyObject::Str(s) => s.chars().nth(sub_idx).map(|c| py_str(&c.to_string())),
                                                            _ => None,
                                                        };
                                                        match sub_obj {
                                                            Some(v) => Ok(v),
                                                            None => Err(PyError::index_error("index out of range")),
                                                        }
                                                    } else {
                                                        Ok(obj.borrow().get_attribute(sub_str).unwrap_or_else(|_| py_none()))
                                                    }
                                                } else {
                                                    return Err(PyError::index_error("index out of range for positional args"));
                                                }
                                            } else {
                                                return Err(PyError::key_error(format!("'{}'", name_part)));
                                            }
                                        } else {
                                            // Named field — bare name (no
                                            // `.attr`/`[index]` sub-access).
                                            kwargs_dict
                                                .as_ref()
                                                .and_then(|d| {
                                                    if let PyObject::Dict(dd) = &*d.borrow() {
                                                        dd.get(&py_str(name_part)).ok().flatten()
                                                    } else {
                                                        None
                                                    }
                                                })
                                                .ok_or_else(|| {
                                                    PyError::key_error(format!("'{}'", name_part))
                                                })
                                        }
                                    };
                                    let val = val?;
                                    // Apply `!conversion` (repr/str/ascii).
                                    let val = match conversion {
                                        Some("r") | Some("a") => py_str(&val.borrow().repr()),
                                        Some("s") => py_str(&val.str()),
                                        _ => val,
                                    };
                                    // Resolve NESTED replacement fields inside
                                    // the spec — `'{:0{}x}'`'s `{}` takes the
                                    // next format arg as the width
                                    // (test_strtod's reference strtod formats
                                    // with `{:0{}x}`). Each nested field
                                    // consumes one arg from the SAME auto/
                                    // manual counters.
                                    let spec = resolve_nested_spec(
                                        spec,
                                        &mut next_auto,
                                        &mut used_manual_numbering,
                                        &mut used_auto_numbering,
                                        pos_args,
                                        kwargs_dict.as_ref(),
                                    )?;
                                    result.push_str(&crate::vm::format_with_spec(&val, &spec)?);
                                } else if c == '}' {
                                    if chars.as_str().starts_with('}') {
                                        result.push('}');
                                        chars.next();
                                    }
                                } else {
                                    result.push(c);
                                }
                            }
                            Ok(py_str(&result))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "split" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "split".to_string(),
                        func: |args| {
                            let s = args[0].str();
                            let sep = if args.len() > 1
                                && !matches!(&*args[1].borrow(), PyObject::None)
                            {
                                Some(args[1].str())
                            } else {
                                None
                            };
                            let maxsplit = if args.len() > 2 {
                                args[2].as_i64().unwrap_or(-1)
                            } else {
                                -1
                            };
                            let parts: Vec<PyObjectRef> = match (sep, maxsplit) {
                                (Some(sep), n) if n >= 0 => {
                                    s.splitn(n as usize + 1, &sep).map(py_str).collect()
                                }
                                (Some(sep), _) => s.split(&sep).map(py_str).collect(),
                                (None, n) if n >= 0 => {
                                    let mut parts: Vec<&str> = Vec::new();
                                    let mut rest = s.as_str();
                                    while parts.len() < n as usize {
                                        let trimmed = rest.trim_start();
                                        if trimmed.is_empty() {
                                            rest = trimmed;
                                            break;
                                        }
                                        match trimmed.find(char::is_whitespace) {
                                            Some(idx) => {
                                                parts.push(&trimmed[..idx]);
                                                rest = &trimmed[idx..];
                                            }
                                            None => {
                                                rest = trimmed;
                                                break;
                                            }
                                        }
                                    }
                                    let tail = rest.trim();
                                    if !tail.is_empty() {
                                        parts.push(tail);
                                    }
                                    parts.into_iter().map(py_str).collect()
                                }
                                (None, _) => s.split_whitespace().map(py_str).collect(),
                            };
                            Ok(py_list(parts))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "rsplit" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "rsplit".to_string(),
                        func: |args| {
                            let s = args[0].str();
                            let sep = if args.len() > 1
                                && !matches!(&*args[1].borrow(), PyObject::None)
                            {
                                Some(args[1].str())
                            } else {
                                None
                            };
                            let maxsplit = if args.len() > 2 {
                                args[2].as_i64().unwrap_or(-1)
                            } else {
                                -1
                            };
                            let parts: Vec<PyObjectRef> = match (sep, maxsplit) {
                                (Some(sep), n) if n >= 0 => {
                                    let mut parts: Vec<&str> =
                                        s.rsplitn(n as usize + 1, &sep).collect();
                                    parts.reverse();
                                    parts.into_iter().map(py_str).collect()
                                }
                                (Some(sep), _) => s.split(&sep).map(py_str).collect(),
                                (None, n) if n >= 0 => {
                                    let mut parts: Vec<&str> = Vec::new();
                                    let mut rest = s.as_str();
                                    while parts.len() < n as usize {
                                        let trimmed = rest.trim_end();
                                        if trimmed.is_empty() {
                                            rest = trimmed;
                                            break;
                                        }
                                        match trimmed.rfind(char::is_whitespace) {
                                            Some(idx) => {
                                                parts.push(&trimmed[idx + 1..]);
                                                rest = &trimmed[..idx];
                                            }
                                            None => {
                                                parts.push(trimmed);
                                                rest = "";
                                                break;
                                            }
                                        }
                                    }
                                    let head = rest.trim();
                                    if !head.is_empty() {
                                        parts.push(head);
                                    }
                                    parts.reverse();
                                    parts.into_iter().map(py_str).collect()
                                }
                                (None, _) => s.split_whitespace().map(py_str).collect(),
                            };
                            Ok(py_list(parts))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "join" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "join".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "join() takes exactly one argument",
                                ));
                            }
                            let sep = args[0].str();
                            // FAST PATH: a plain List of strs -- single
                            // allocation with precomputed capacity, no
                            // per-item iterator dispatch, no String clones.
                            if let PyObject::List(items) = &*args[1].borrow() {
                                let mut cap = sep.len().saturating_mul(items.len().saturating_sub(1));
                                for it in items.iter() {
                                    let b = it.borrow();
                                    match &*b {
                                        PyObject::Str(sv) => cap += sv.len(),
                                        _ => {
                                            return Err(PyError::type_error(format!(
                                                "sequence item {}: expected str instance, {} found",
                                                items.iter().position(|x| std::ptr::eq(x, it)).map(|_| 0).unwrap_or(0),
                                                b.type_name()
                                            )));
                                        }
                                    }
                                }
                                let mut out = String::with_capacity(cap);
                                for (i, it) in items.iter().enumerate() {
                                    if i > 0 {
                                        out.push_str(&sep);
                                    }
                                    if let PyObject::Str(sv) = &*it.borrow() {
                                        out.push_str(sv);
                                    }
                                }
                                return Ok(py_str(&out));
                            }
                            // Real `str.join` accepts any iterable, not just a
                            // list (tuples/generators/dict_keys/etc. are all
                            // common in real code, e.g. `''.join(chunk for
                            // chunk in parts)`), so materialize through the
                            // normal iterator protocol instead of only
                            // recognizing a literal `PyObject::List`.
                            let iterator = crate::object::builtin_iter(&[args[1].clone()])?;
                            let mut parts: Vec<String> = Vec::new();
                            loop {
                                match crate::object::builtin_next(&[iterator.clone()]) {
                                    Ok(v) => {
                                        // join requires str items (CPython:
                                        // 'sequence item N: expected str
                                        // instance, int found').
                                        if !matches!(&*v.borrow(), PyObject::Str(_)) {
                                            return Err(PyError::type_error(format!(
                                                "sequence item {}: expected str instance, {} found",
                                                parts.len(),
                                                v.borrow().type_name()
                                            )));
                                        }
                                        parts.push(v.str());
                                    }
                                    Err(PyError::StopIteration) => break,
                                    Err(e) => return Err(e),
                                }
                            }
                            Ok(py_str(&parts.join(&sep)))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "upper" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "upper".to_string(),
                        func: |args| {
                            if args.len() != 1 {
                                return Err(PyError::type_error(
                                    "upper() takes no arguments (1 given)",
                                ));
                            }
                            Ok(py_str(&args[0].str().to_uppercase()))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "lower" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "lower".to_string(),
                        func: |args| {
                            if args.len() != 1 {
                                return Err(PyError::type_error(
                                    "lower() takes no arguments (1 given)",
                                ));
                            }
                            Ok(py_str(&lower_with_final_sigma(&args[0].str())))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "strip" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "strip".to_string(),
                        func: |args| {
                            if args.len() > 2 {
                                return Err(PyError::type_error(
                                    "strip() takes at most 1 argument (2 given)",
                                ));
                            }
                            if args.len() == 2
                                && !matches!(&*args[1].borrow(), PyObject::Str(_) | PyObject::None)
                            {
                                return Err(PyError::type_error(format!(
                                    "strip() argument must be str or None, not {}",
                                    args[1].borrow().type_name(),
                                )));
                            }
                            let chars = if args.len() > 1 {
                                if let PyObject::None = &*args[1].borrow() {
                                    " \t\n\r\x0b\x0c".to_string()
                                } else {
                                    args[1].str()
                                }
                            } else {
                                " \t\n\r\x0b\x0c".to_string()
                            };
                            Ok(py_str(
                                args[0].str().trim_matches(|c: char| chars.contains(c)),
                            ))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "lstrip" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "lstrip".to_string(),
                        func: |args| {
                            if args.len() > 2 {
                                return Err(PyError::type_error(
                                    "lstrip() takes at most 1 argument (2 given)",
                                ));
                            }

                            let chars = if args.len() > 1 {
                                args[1].str()
                            } else {
                                " \t\n\r\x0b\x0c".to_string()
                            };
                            Ok(py_str(
                                args[0]
                                    .str()
                                    .trim_start_matches(|c: char| chars.contains(c)),
                            ))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "rstrip" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "rstrip".to_string(),
                        func: |args| {
                            if args.len() > 2 {
                                return Err(PyError::type_error(
                                    "rstrip() takes at most 1 argument (2 given)",
                                ));
                            }

                            let chars = if args.len() > 1 {
                                args[1].str()
                            } else {
                                " \t\n\r\x0b\x0c".to_string()
                            };
                            Ok(py_str(
                                args[0].str().trim_end_matches(|c: char| chars.contains(c)),
                            ))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "startswith" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "startswith".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "startswith() takes at least 1 argument",
                                ));
                            }
                            let start = opt_i64_arg(args.get(2));
                            let end = opt_i64_arg(args.get(3));
                            let prefixes: Vec<String> = match &*args[1].borrow() {
                                PyObject::Tuple(items) => items.iter().map(|x| x.str()).collect(),
                                _ => vec![args[1].str()],
                            };
                            // Borrow the haystack's content directly instead of
                            // `.str()` (which always returns a freshly-cloned
                            // owned `String`) — avoids an O(n) copy on EVERY
                            // call, same reason as `char_slice_with_start`'s own
                            // doc comment (this method is commonly called in a
                            // tight loop with an explicit start index).
                            let obj0 = args[0].borrow();
                            let s: &str = match &*obj0 {
                                PyObject::Str(cs) => cs.as_str(),
                                _ => return Err(PyError::runtime_error("startswith on non-str")),
                            };
                            let (_, sub) = char_slice_with_start(s, start, end);
                            Ok(py_bool(
                                prefixes.iter().any(|p| sub.starts_with(p.as_str())),
                            ))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "endswith" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "endswith".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "endswith() takes at least 1 argument",
                                ));
                            }
                            let start = opt_i64_arg(args.get(2));
                            let end = opt_i64_arg(args.get(3));
                            let suffixes: Vec<String> = match &*args[1].borrow() {
                                PyObject::Tuple(items) => items.iter().map(|x| x.str()).collect(),
                                _ => vec![args[1].str()],
                            };
                            let obj0 = args[0].borrow();
                            let s: &str = match &*obj0 {
                                PyObject::Str(cs) => cs.as_str(),
                                _ => return Err(PyError::runtime_error("endswith on non-str")),
                            };
                            let (_, sub) = char_slice_with_start(s, start, end);
                            Ok(py_bool(suffixes.iter().any(|p| sub.ends_with(p.as_str()))))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "find" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "find".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "find() takes at least 1 argument",
                                ));
                            }
                            let s = args[0].str();
                            if !matches!(&*args[1].borrow(), PyObject::Str(_)) {
                                return Err(PyError::type_error(format!(
                                    "find() argument must be str, not {}",
                                    args[1].borrow().type_name(),
                                )));
                            }

                            let needle = args[1].str();
                            let start = opt_i64_arg(args.get(2));
                            let end = opt_i64_arg(args.get(3));
                            Ok(py_int(
                                str_find_impl(&s, &needle, start, end, false)
                                    .map(|i| i as i64)
                                    .unwrap_or(-1),
                            ))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "rfind" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "rfind".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "rfind() takes at least 1 argument",
                                ));
                            }
                            let s = args[0].str();
                            if !matches!(&*args[1].borrow(), PyObject::Str(_)) {
                                return Err(PyError::type_error(format!(
                                    "rfind() argument must be str, not {}",
                                    args[1].borrow().type_name(),
                                )));
                            }

                            let needle = args[1].str();
                            let start = opt_i64_arg(args.get(2));
                            let end = opt_i64_arg(args.get(3));
                            Ok(py_int(
                                str_find_impl(&s, &needle, start, end, true)
                                    .map(|i| i as i64)
                                    .unwrap_or(-1),
                            ))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "index" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "index".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "index() takes at least 1 argument",
                                ));
                            }
                            let s = args[0].str();
                            if !matches!(&*args[1].borrow(), PyObject::Str(_)) {
                                return Err(PyError::type_error(format!(
                                    "index() argument must be str, not {}",
                                    args[1].borrow().type_name(),
                                )));
                            }

                            let needle = args[1].str();
                            let start = opt_i64_arg(args.get(2));
                            let end = opt_i64_arg(args.get(3));
                            str_find_impl(&s, &needle, start, end, false)
                                .map(|i| py_int(i as i64))
                                .ok_or_else(|| PyError::value_error("substring not found"))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "rindex" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "rindex".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "rindex() takes at least 1 argument",
                                ));
                            }
                            let s = args[0].str();
                            if !matches!(&*args[1].borrow(), PyObject::Str(_)) {
                                return Err(PyError::type_error(format!(
                                    "rindex() argument must be str, not {}",
                                    args[1].borrow().type_name(),
                                )));
                            }

                            let needle = args[1].str();
                            let start = opt_i64_arg(args.get(2));
                            let end = opt_i64_arg(args.get(3));
                            str_find_impl(&s, &needle, start, end, true)
                                .map(|i| py_int(i as i64))
                                .ok_or_else(|| PyError::value_error("substring not found"))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "count" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "count".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "count() takes at least 1 argument",
                                ));
                            }
                            let s = args[0].str();
                            if !matches!(&*args[1].borrow(), PyObject::Str(_)) {
                                return Err(PyError::type_error(format!(
                                    "count() argument must be str, not {}",
                                    args[1].borrow().type_name(),
                                )));
                            }

                            let needle = args[1].str();
                            let start = opt_i64_arg(args.get(2));
                            let end = opt_i64_arg(args.get(3));
                            let chars: Vec<char> = s.chars().collect();
                            let (st, en) = resolve_str_slice_bounds(chars.len(), start, end);
                            let sub: String = chars[st..en].iter().collect();
                            let c = if needle.is_empty() {
                                sub.chars().count() + 1
                            } else {
                                sub.matches(needle.as_str()).count()
                            };
                            Ok(py_int(c as i64))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    _ => return super::str2::get(o, name),
                }
            }
        _ => unreachable!("{} called with non-{} object", stringify!(get), o.type_name()),
    }
}
