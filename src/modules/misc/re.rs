use crate::object::*;
use std::collections::HashMap;

mod compile;
pub use compile::*;

mod match_obj;
pub use match_obj::*;

pub fn create_re_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! re_func {
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

    re_func!("search", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("search() takes at least 2 arguments"));
        }
        let pattern = args[0].str();
        let string = args[1].str();
        match compile_python_regex(&pattern) {
            Ok(re) => {
                let caps = re.captures(&string).unwrap_or(None);
                Ok(make_match_object_detailed(&re, caps, &string, &pattern, 0, 0, string.len()))
            }
            Err(e) => Err(re_pattern_error(map_regex_error(&e.to_string()), Some(pattern.clone()), None)),
        }
    });

    re_func!("match", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("match() takes at least 2 arguments"));
        }
        let pattern = args[0].str();
        let string = args[1].str();
        match compile_python_regex(&pattern) {
            Ok(re) => {
                let caps = re.captures(&string).unwrap_or(None);
                // Only succeed if match starts at position 0
                let result = match caps {
                    Some(c) if c.get(0).map(|m| m.start()) == Some(0) => Some(c),
                    _ => None,
                };
                Ok(make_match_object_detailed(&re, result, &string, &pattern, 0, 0, string.len()))
            }
            Err(e) => Err(re_pattern_error(map_regex_error(&e.to_string()), Some(pattern.clone()), None)),
        }
    });

    re_func!("fullmatch", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error(
                "fullmatch() takes at least 2 arguments",
            ));
        }
        let pattern = args[0].str();
        let string = args[1].str();
        match compile_python_regex(&pattern) {
            Ok(re) => {
                let caps = re.captures(&string).unwrap_or(None).filter(|c| {
                    c.get(0)
                        .map(|m| m.start() == 0 && m.end() == string.len())
                        .unwrap_or(false)
                });
                Ok(make_match_object_detailed(&re, caps, &string, &pattern, 0, 0, string.len()))
            }
            Err(e) => Err(re_pattern_error(map_regex_error(&e.to_string()), Some(pattern.clone()), None)),
        }
    });

    re_func!("findall", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("findall() takes at least 2 arguments"));
        }
        let pattern = args[0].str();
        let string = args[1].str();
        match compile_python_regex(&pattern) {
            Ok(re) => {
                let results: Vec<PyObjectRef> = re
                    .find_iter(&string)
                    .filter_map(|r| r.ok())
                    .map(|m| py_str(m.as_str()))
                    .collect();
                Ok(py_list(results))
            }
            Err(e) => Err(re_pattern_error(map_regex_error(&e.to_string()), Some(pattern.clone()), None)),
        }
    });

    re_func!("sub", |args| {
        if args.len() < 3 {
            return Err(PyError::type_error("sub() takes at least 3 arguments"));
        }
        // Real `re.sub` accepts EITHER a string template (`\1`/`\g<name>`
        // backreferences) OR a callable taking the `Match` and returning
        // the replacement string — previously `repl` was unconditionally
        // stringified via `.str()`, so a callable replacement (a very
        // common idiom — e.g. `html.unescape`'s own `_charref.sub
        // (_replace_charref, s)`) was never actually CALLED: its `str()`
        // (something like `<function _replace_charref>`) was substituted
        // in literally instead. Both branches now share one manual
        // match-iteration loop (rather than `replace_all`, which can't
        // invoke a callback) — this also adds real `count` support (the
        // 4th positional arg), which the previous `replace_all`-based
        // version silently ignored entirely.
        let pattern = args[0].str();
        let is_callable_repl = !matches!(&*args[1].borrow(), PyObject::Str(_));
        let repl_template = if is_callable_repl {
            String::new()
        } else {
            translate_python_replacement(&args[1].str())
        };
        let string = args[2].str();
        // `count`/`flags` may arrive positionally (args[3]) OR as a trailing
        // kwargs dict (`re.sub(p, r, s, count=1)` — a real, common call
        // shape; this project's calling convention appends a `{"count":
        // ..., "flags": ...}` dict as the final positional arg for keyword
        // calls, same as `sorted`'s own `key=`/`reverse=` handling).
        let count = if args.len() > 3 {
            if let PyObject::Dict(kwargs) = &*args[3].borrow() {
                kwargs
                    .get(&py_str("count"))
                    .ok()
                    .flatten()
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0)
            } else {
                args[3].as_i64().unwrap_or(0)
            }
        } else {
            0
        };
        match compile_python_regex(&pattern) {
            Ok(re) => {
                if !is_callable_repl {
                    if let Err(e) = validate_g_template(&args[1].str(), &re) {
                        return Err(e);
                    }
                    // Check for unknown group name like \g<ab> where ab not in pattern - should raise IndexError
                    let repl_str = args[1].str();
                    for cap in re.capture_names().filter_map(|n| n) {
                        let _ = cap;
                    }
                    // Simple check: if repl contains \g<ab> and ab not in re's group names, raise IndexError
                    let mut search_idx = 0;
                    let repl_chars: Vec<char> = repl_str.chars().collect();
                    while search_idx < repl_chars.len() {
                        if repl_chars[search_idx] == '\\' && search_idx + 2 < repl_chars.len() && repl_chars[search_idx+1] == 'g' && repl_chars[search_idx+2] == '<' {
                            let mut j = search_idx + 3;
                            while j < repl_chars.len() && repl_chars[j] != '>' {
                                j += 1;
                            }
                            if j < repl_chars.len() {
                                let name: String = repl_chars[search_idx+3..j].iter().collect();
                                if !name.is_empty() && !name.chars().next().unwrap().is_ascii_digit() && !name.chars().all(|c| c.is_ascii_digit()) {
                                    let found = re.capture_names().any(|n| n == Some(name.as_str()));
                                    if !found {
                                        return Err(PyError::IndexError(format!("unknown group name '{}'", name)));
                                    }
                                }
                            }
                        }
                        search_idx += 1;
                    }
                }
                let mut result = String::new();
                let mut last_end = 0usize;
                let mut n = 0i64;
                for caps in re.captures_iter(&string) {
                    let caps = match caps {
                        Ok(c) => c,
                        Err(_) => break,
                    };
                    if count > 0 && n >= count {
                        break;
                    }
                    let (m_start, m_end) = {
                        let m = caps.get(0).unwrap();
                        (m.start(), m.end())
                    };
                    if m_start < last_end {
                        continue;
                    }
                    result.push_str(&string[last_end..m_start]);
                    if is_callable_repl {
                        // Calling an arbitrary Python callable (not just a
                        // native `BuiltinFunction`) from within a native
                        // function's own body needs `call_bound_method`'s
                        // "disposable VM" path (for a plain `PyObject::
                        // Function`) — reentering `vm.call_function` on the
                        // SAME live `VirtualMachine` via `with_vm_mut`
                        // (tried first) corrupted execution state instead:
                        // the callback itself ran and returned correctly,
                        // but control never made it back to the `sub()`
                        // caller afterward (confirmed via a minimal repro —
                        // `print()` calls placed after the `re.sub(...)`
                        // call simply never ran, no error, exit code 0).
                        // Matches the same pattern `sorted(key=...)` already
                        // uses for its own key-function callback.
                        let match_obj = make_match_object_detailed(&re, Some(caps), &string, &pattern, 0, m_start, string.len());
                        let replaced = call_bound_method(args[1].clone(), match_obj, vec![])?;
                        result.push_str(&replaced.str());
                    } else {
                        let mut expanded = String::new();
                        caps.expand(&repl_template, &mut expanded);
                        result.push_str(&expanded);
                    }
                    last_end = m_end;
                    n += 1;
                }
                result.push_str(&string[last_end..]);
                Ok(py_str(&result))
            }
            Err(e) => Err(re_pattern_error(map_regex_error(&e.to_string()), Some(pattern.clone()), None)),
        }
    });

    re_func!("split", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("split() takes at least 2 arguments"));
        }
        // Support `re.split(compiled_pat, string)` where pattern is already CompiledRegex
        let is_compiled = matches!(&*args[0].borrow(), PyObject::CompiledRegex { .. });
        let (re, string, maxsplit) = if is_compiled {
            let re = if let PyObject::CompiledRegex { regex, .. } = &*args[0].borrow() {
                (**regex).clone()
            } else {
                unreachable!()
            };
            let string = args[1].str();
            let maxsplit = {
                let has_kwargs = args.last().map_or(false, |a| matches!(&*a.borrow(), PyObject::Dict(_)));
                if has_kwargs {
                    if let PyObject::Dict(d) = &*args.last().unwrap().borrow() {
                        d.get(&py_str("maxsplit")).ok().flatten().and_then(|v| v.as_i64()).unwrap_or(0) as usize
                    } else { 0 }
                } else if args.len() > 2 {
                    args[2].as_i64().unwrap_or(0) as usize
                } else { 0 }
            };
            (re, string, maxsplit)
        } else {
            let pattern = args[0].str();
            let has_kwargs = args.last().map_or(false, |a| matches!(&*a.borrow(), PyObject::Dict(_)));
            let (string, maxsplit, flags) = if has_kwargs {
                let d = if let PyObject::Dict(d) = &*args.last().unwrap().borrow() { d.clone() } else { Box::new(crate::object::PyDict::new()) };
                let ms = d.get(&py_str("maxsplit")).ok().flatten().and_then(|v| v.as_i64()).unwrap_or(0) as usize;
                let fl = d.get(&py_str("flags")).ok().flatten().and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                (args[1].str(), ms, fl)
            } else {
                let ms = if args.len() > 2 { args[2].as_i64().unwrap_or(0) as usize } else { 0 };
                let fl = if args.len() > 3 { args[3].as_i64().unwrap_or(0) as i32 } else { 0 };
                (args[1].str(), ms, fl)
            };
            let re = match compile_python_regex_flags(&pattern, flags) {
                Ok(r) => r,
                Err(e) => return Err(re_pattern_error(map_regex_error(&e.to_string()), Some(pattern.clone()), None)),
            };
            (re, string, maxsplit)
        };
        // Python's re.split includes captured groups; manual loop over captures
        let mut result: Vec<PyObjectRef> = Vec::new();
        let mut last_end = 0usize;
        let mut n = 0usize;
        let mut caps_iter = re.captures_iter(&string);
        while let Some(caps_res) = caps_iter.next() {
            let caps = match caps_res { Ok(c) => c, Err(_) => break };
            if maxsplit != 0 && n >= maxsplit { break; }
            let m = caps.get(0).unwrap();
            let (s, e) = (m.start(), m.end());
            if s == e {
                if s == string.len() { break; }
                if s == last_end {
                    result.push(py_str(&string[last_end..s]));
                    for i in 1..caps.len() {
                        if let Some(g) = caps.get(i) { result.push(py_str(g.as_str())); } else { result.push(py_none()); }
                    }
                    let next_len = string[s..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
                    last_end = s + next_len;
                    n += 1;
                    continue;
                }
            }
            result.push(py_str(&string[last_end..s]));
            for i in 1..caps.len() {
                if let Some(g) = caps.get(i) { result.push(py_str(g.as_str())); } else { result.push(py_none()); }
            }
            last_end = e;
            n += 1;
        }
        result.push(py_str(&string[last_end..]));
        Ok(py_list(result))
    });

    re_func!("compile", |args| {
        if args.len() < 1 {
            return Err(PyError::type_error("compile() takes at least 1 argument"));
        }
        // Compiling an already-compiled pattern returns it unchanged
        // (CPython: re.compile(re.compile('x')) is a no-op).
        if matches!(&*args[0].borrow(), PyObject::CompiledRegex { .. }) {
            return Ok(args[0].clone());
        }
        let pattern = args[0].str();
        let flags = if args.len() > 1 {
            args[1].as_i64().unwrap_or(0) as i32
        } else {
            0
        };
        match compile_python_regex_flags(&pattern, flags) {
            Ok(re) => Ok(PyObjectRef::new(PyObject::CompiledRegex {
                regex: Box::new(re),
                pattern: pattern.to_string(),
                flags,
            })),
            Err(e) => Err(re_pattern_error(map_regex_error(&e.to_string()), Some(pattern.clone()), None)),
        }
    });

    // finditer — returns a list of Match objects (not a lazy iterator, but sufficient for Django)
    re_func!("finditer", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("finditer() takes at least 2 arguments"));
        }
        let pattern = args[0].str();
        let string = args[1].str();
        match compile_python_regex(&pattern) {
            Ok(re) => {
                let matches: Vec<PyObjectRef> = re
                    .captures_iter(&string)
                    .filter_map(|r| r.ok())
                    .map(|c| {
                        let start = c.get(0).map(|m| m.start()).unwrap_or(0);
                        make_match_object_detailed(&re, Some(c), &string, &pattern, 0, start, string.len())
                    })
                    .collect();
                // Return a list that can be iterated over
                Ok(py_list(matches))
            }
            Err(e) => Err(re_pattern_error(map_regex_error(&e.to_string()), Some(pattern.clone()), None)),
        }
    });

    re_func!("escape", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("escape() missing required argument"));
        }
        let s = args[0].str();
        let escaped = regex::escape(&s);
        Ok(py_str(&escaped))
    });

    // Regex compile flags
    d.insert_str("IGNORECASE", py_int(2));
    d.insert_str("ASCII", py_int(256));
    d.insert_str("DOTALL", py_int(16));
    d.insert_str("MULTILINE", py_int(8));
    d.insert_str("VERBOSE", py_int(64));
    d.insert_str("LOCALE", py_int(4));
    d.insert_str("UNICODE", py_int(32));
    d.insert_str("TEMPLATE", py_int(1));
    d.insert_str("DEBUG", py_int(128));
    d.insert_str("NOFLAG", py_int(0));
    d.insert_str("I", py_int(2));
    d.insert_str("A", py_int(256));
    d.insert_str("S", py_int(16));
    d.insert_str("M", py_int(8));
    d.insert_str("X", py_int(64));
    d.insert_str("L", py_int(4));
    d.insert_str("U", py_int(32));
    d.insert_str("T", py_int(1));

    // RegexFlag — IntFlag-like type with same members as module-level flags
    {
        let mut rf_dict = HashMap::new();
        rf_dict.insert("NOFLAG".to_string(), py_int(0));
        rf_dict.insert("ASCII".to_string(), py_int(256));
        rf_dict.insert("A".to_string(), py_int(256));
        rf_dict.insert("IGNORECASE".to_string(), py_int(2));
        rf_dict.insert("I".to_string(), py_int(2));
        rf_dict.insert("LOCALE".to_string(), py_int(4));
        rf_dict.insert("L".to_string(), py_int(4));
        rf_dict.insert("UNICODE".to_string(), py_int(32));
        rf_dict.insert("U".to_string(), py_int(32));
        rf_dict.insert("MULTILINE".to_string(), py_int(8));
        rf_dict.insert("M".to_string(), py_int(8));
        rf_dict.insert("DOTALL".to_string(), py_int(16));
        rf_dict.insert("S".to_string(), py_int(16));
        rf_dict.insert("VERBOSE".to_string(), py_int(64));
        rf_dict.insert("X".to_string(), py_int(64));
        rf_dict.insert("TEMPLATE".to_string(), py_int(1));
        rf_dict.insert("T".to_string(), py_int(1));
        rf_dict.insert("DEBUG".to_string(), py_int(128));
        let rf_type = PyObjectRef::new(PyObject::Type {
            name: "RegexFlag".to_string(),
            dict: Box::new(str_map_to_typedict(rf_dict)),
            bases: vec![],
            mro: vec![],
        });
        if let PyObject::Type { mro, .. } = &mut *rf_type.borrow_mut() {
            *mro = vec![rf_type.clone()];
        }
        d.insert_str("RegexFlag", rf_type);
    }

    // re.error / re.PatternError — alias, same object
    {
        let pattern_error = PyObjectRef::new(PyObject::BuiltinFunction {
            name: "PatternError".to_string(),
            func: |args| {
                let msg = args.get(0).map(|a| a.str()).unwrap_or_default();
                let pattern = args.get(1).cloned().unwrap_or_else(py_none);
                let pos = args.get(2).cloned().unwrap_or_else(py_none);
                let mut extra = HashMap::new();
                extra.insert("msg".to_string(), py_str(&msg));
                extra.insert("pattern".to_string(), pattern);
                extra.insert("pos".to_string(), pos);
                Ok(PyObjectRef::new(PyObject::Exception {
                    typ: "PatternError".to_string(),
                    args: vec![py_str(&msg)],
                    cause: None,
                    suppress_context: false,
                    context: None,
                    traceback: None,
                    extra: Some(extra),
                }))
            },
        });
        d.insert_str("error", pattern_error.clone());
        d.insert_str("PatternError", pattern_error);
    }

    // purge — clear cache (no-op)
    re_func!("purge", |_| Ok(py_none()));

    // subn — like sub but returns (new_string, count)
    re_func!("subn", |args| {
        if args.len() < 3 {
            return Err(PyError::type_error("subn() takes at least 3 arguments"));
        }
        let pattern = args[0].str();
        let is_callable_repl = !matches!(&*args[1].borrow(), PyObject::Str(_));
        let repl_template = if is_callable_repl {
            String::new()
        } else {
            translate_python_replacement(&args[1].str())
        };
        let string = args[2].str();
        let count = if args.len() > 3 {
            if let PyObject::Dict(kwargs) = &*args[3].borrow() {
                kwargs
                    .get(&py_str("count"))
                    .ok()
                    .flatten()
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0)
            } else {
                args[3].as_i64().unwrap_or(0)
            }
        } else {
            0
        };
        match compile_python_regex(&pattern) {
            Ok(re) => {
                if !is_callable_repl {
                    if let Err(e) = validate_g_template(&args[1].str(), &re) {
                        return Err(e);
                    }
                    // Check for unknown group name like \g<ab> where ab not in pattern - should raise IndexError
                    let repl_str = args[1].str();
                    for cap in re.capture_names().filter_map(|n| n) {
                        let _ = cap;
                    }
                    // Simple check: if repl contains \g<ab> and ab not in re's group names, raise IndexError
                    let mut search_idx = 0;
                    let repl_chars: Vec<char> = repl_str.chars().collect();
                    while search_idx < repl_chars.len() {
                        if repl_chars[search_idx] == '\\' && search_idx + 2 < repl_chars.len() && repl_chars[search_idx+1] == 'g' && repl_chars[search_idx+2] == '<' {
                            let mut j = search_idx + 3;
                            while j < repl_chars.len() && repl_chars[j] != '>' {
                                j += 1;
                            }
                            if j < repl_chars.len() {
                                let name: String = repl_chars[search_idx+3..j].iter().collect();
                                if !name.is_empty() && !name.chars().next().unwrap().is_ascii_digit() && !name.chars().all(|c| c.is_ascii_digit()) {
                                    let found = re.capture_names().any(|n| n == Some(name.as_str()));
                                    if !found {
                                        return Err(PyError::IndexError(format!("unknown group name '{}'", name)));
                                    }
                                }
                            }
                        }
                        search_idx += 1;
                    }
                }
                let mut result = String::new();
                let mut last_end = 0usize;
                let mut n = 0i64;
                for caps in re.captures_iter(&string) {
                    let caps = match caps {
                        Ok(c) => c,
                        Err(_) => break,
                    };
                    if count > 0 && n >= count {
                        break;
                    }
                    let (m_start, m_end) = {
                        let m = caps.get(0).unwrap();
                        (m.start(), m.end())
                    };
                    if m_start < last_end {
                        continue;
                    }
                    result.push_str(&string[last_end..m_start]);
                    if is_callable_repl {
                        let match_obj = make_match_object_detailed(&re, Some(caps), &string, &pattern, 0, m_start, string.len());
                        let replaced = call_bound_method(args[1].clone(), match_obj, vec![])?;
                        result.push_str(&replaced.str());
                    } else {
                        let mut expanded = String::new();
                        caps.expand(&repl_template, &mut expanded);
                        result.push_str(&expanded);
                    }
                    last_end = m_end;
                    n += 1;
                }
                result.push_str(&string[last_end..]);
                Ok(py_tuple(vec![py_str(&result), py_int(n)]))
            }
            Err(e) => Err(re_pattern_error(map_regex_error(&e.to_string()), Some(pattern.clone()), None)),
        }
    });

    // Scanner — tokenizer helper
    {
        let mut scanner_dict = HashMap::new();
        scanner_dict.insert(
            "__init__".to_string(),
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "__init__".to_string(),
                func: |args| {
                    if args.len() < 2 {
                        return Err(PyError::type_error("Scanner.__init__ missing lexicon"));
                    }
                    let self_obj = &args[0];
                    let lexicon = args[1].clone();
                    let flags = args.get(2).and_then(|a| a.as_i64()).unwrap_or(0) as i32;
                    let items: Vec<PyObjectRef> = if let PyObject::List(v) | PyObject::Tuple(v) = &*lexicon.borrow() {
                        v.clone()
                    } else {
                        return Err(PyError::type_error("lexicon must be list/tuple"));
                    };
                    let mut compiled_list: Vec<PyObjectRef> = Vec::new();
                    for item in &items {
                        let pair: Vec<PyObjectRef> = if let PyObject::Tuple(v) | PyObject::List(v) = &*item.borrow() {
                            v.clone()
                        } else {
                            continue;
                        };
                        if pair.len() < 2 {
                            continue;
                        }
                        let pat = pair[0].str();
                        match compile_python_regex_flags(&pat, flags) {
                            Ok(r) => {
                                compiled_list.push(PyObjectRef::new(PyObject::CompiledRegex {
                                    regex: Box::new(r),
                                    pattern: pat,
                                    flags,
                                }));
                            }
                            Err(e) => return Err(re_pattern_error(map_regex_error(&e.to_string()), Some(pat), None)),
                        }
                    }
                    let dummy_re = fancy_regex::Regex::new("").unwrap();
                    let dummy = PyObjectRef::new(PyObject::CompiledRegex {
                        regex: Box::new(dummy_re),
                        pattern: "".to_string(),
                        flags: 0,
                    });
                    if let PyObject::Instance { dict, .. } = &mut *self_obj.borrow_mut() {
                        dict.insert_str("lexicon", lexicon.clone());
                        dict.insert_str("_compiled", py_list(compiled_list));
                        dict.insert_str("scanner", dummy);
                        dict.insert_str("match", py_none());
                    }
                    Ok(py_none())
                },
            }),
        );
        scanner_dict.insert(
            "scan".to_string(),
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "scan".to_string(),
                func: |args| {
                    if args.is_empty() {
                        return Err(PyError::type_error("scan() missing self"));
                    }
                    let self_obj = &args[0];
                    let string = args.get(1).map(|a| a.str()).unwrap_or_default();
                    let lexicon = self_obj.borrow().get_attribute("lexicon")
                        .unwrap_or_else(|_| py_list(vec![]));
                    let compiled = self_obj.borrow().get_attribute("_compiled")
                        .unwrap_or_else(|_| py_list(vec![]));
                    let lex_items: Vec<PyObjectRef> = if let PyObject::List(v) | PyObject::Tuple(v) = &*lexicon.borrow() { v.clone() } else { vec![] };
                    let comp_items: Vec<PyObjectRef> = if let PyObject::List(v) = &*compiled.borrow() { v.clone() } else { vec![] };
                    let mut result: Vec<PyObjectRef> = Vec::new();
                    let mut i: usize = 0;
                    let bytes_len = string.len();
                    // To avoid infinite loops, track if progress is made
                    while i < bytes_len {
                        let mut matched = false;
                        for (idx, comp) in comp_items.iter().enumerate() {
                            let re = if let PyObject::CompiledRegex { regex, .. } = &*comp.borrow() {
                                (**regex).clone()
                            } else { continue };
                            let caps_opt = re.captures_from_pos(&string, i).unwrap_or(None);
                            if let Some(caps) = caps_opt {
                                if let Some(m) = caps.get(0) {
                                    if m.start() != i {
                                        continue;
                                    }
                                    let j = m.end();
                                    if i == j {
                                        continue;
                                    }
                                    let action = lex_items.get(idx)
                                        .and_then(|item| {
                                            if let PyObject::Tuple(v) | PyObject::List(v) = &*item.borrow() {
                                                v.get(1).cloned()
                                            } else { None }
                                        })
                                        .unwrap_or_else(py_none);
                                    let is_none = matches!(&*action.borrow(), PyObject::None);
                                    let mut use_action = true;
                                    let mut final_val: Option<PyObjectRef> = None;
                                    if is_none {
                                        use_action = false;
                                    } else if !matches!(&*action.borrow(), PyObject::None) {
                                        // Check if callable (Function, BuiltinFunction, Closure, etc.)
                                        let is_callable = {
                                            let b = action.borrow();
                                            matches!(&*b, PyObject::Function(_) | PyObject::BuiltinFunction{..} | PyObject::Closure(_) | PyObject::Instance{..} | PyObject::BuiltinMethod{..})
                                        };
                                        if is_callable {
                                            let token_str = py_str(m.as_str());
                                            // set self.match for the callable to inspect
                                            let caps_clone = re.captures_from_pos(&string, i).unwrap_or(None);
                                            let match_obj = make_match_object_detailed(&re, caps_clone, &string, "", 0, i, string.len());
                                            if let PyObject::Instance { dict, .. } = &mut *self_obj.borrow_mut() {
                                                dict.insert_str("match", match_obj.clone());
                                            }
                                            // CPython calls action(scanner, token)
                                            match call_bound_method(action.clone(), self_obj.clone(), vec![token_str.clone()]) {
                                                Ok(v) => {
                                                    if matches!(&*v.borrow(), PyObject::None) {
                                                        use_action = false;
                                                    } else {
                                                        final_val = Some(v);
                                                    }
                                                }
                                                Err(_) => {
                                                    // Fallback: action(token) single-arg form
                                                    match call_bound_method(action.clone(), token_str.clone(), vec![]) {
                                                        Ok(v2) => {
                                                            if matches!(&*v2.borrow(), PyObject::None) {
                                                                use_action = false;
                                                            } else {
                                                                final_val = Some(v2);
                                                            }
                                                        }
                                                        Err(e) => return Err(e),
                                                    }
                                                }
                                            }
                                        } else {
                                            final_val = Some(action.clone());
                                        }
                                    }
                                    if use_action {
                                        if let Some(v) = final_val {
                                            result.push(v);
                                        }
                                    }
                                    i = j;
                                    matched = true;
                                    break;
                                }
                            }
                        }
                        if !matched {
                            break;
                        }
                    }
                    let rest = py_str(&string[i..]);
                    Ok(py_tuple(vec![py_list(result), rest]))
                },
            }),
        );
        let scanner_type = PyObjectRef::new(PyObject::Type {
            name: "Scanner".to_string(),
            dict: Box::new(str_map_to_typedict(scanner_dict)),
            bases: vec![],
            mro: vec![],
        });
        if let PyObject::Type { mro, .. } = &mut *scanner_type.borrow_mut() {
            *mro = vec![scanner_type.clone()];
        }
        d.insert_str("Scanner", scanner_type);
    }

    // re.Pattern and re.Match type stubs (needed by typing and type-checking code)
    let re_pattern_type = PyObjectRef::new(PyObject::Type {
        name: "Pattern".to_string(),
        dict: Box::new(str_map_to_typedict(HashMap::new())),
        bases: vec![],
        mro: vec![],
    });
    d.insert_str("Pattern", re_pattern_type);

    let re_match_type = PyObjectRef::new(PyObject::Type {
        name: "Match".to_string(),
        dict: Box::new(str_map_to_typedict(HashMap::new())),
        bases: vec![],
        mro: vec![],
    });
    d.insert_str("Match", re_match_type);

    d.insert_str("__version__", py_str("2.2.1"));
    d.insert_str("__all__", py_list(vec![
        py_str("match"), py_str("fullmatch"), py_str("search"), py_str("sub"), py_str("subn"),
        py_str("split"), py_str("findall"), py_str("finditer"), py_str("compile"), py_str("purge"),
        py_str("escape"), py_str("error"), py_str("Pattern"), py_str("Match"), py_str("A"),
        py_str("I"), py_str("L"), py_str("M"), py_str("S"), py_str("X"), py_str("U"),
        py_str("ASCII"), py_str("IGNORECASE"), py_str("LOCALE"), py_str("MULTILINE"),
        py_str("DOTALL"), py_str("VERBOSE"), py_str("UNICODE"), py_str("NOFLAG"),
        py_str("RegexFlag"), py_str("PatternError"),
    ]));

    d
}
