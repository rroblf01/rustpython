use crate::object::*;
use std::collections::HashMap;

use super::compile::translate_python_replacement;
/// Resolve a `group()`/`start()`/`end()` group argument (an int index, OR a
/// string/name — real `re.Match` accepts both) against the match's stored
/// `_group_names` dict (name -> 1-based index), returning a 0-based index
/// into `_starts`/`_ends`/`_groups_text` (0 = whole match). Returns `None`
/// for a name that doesn't exist (caller raises `IndexError`, matching
/// real CPython's `no such group`).
pub fn resolve_group_arg(obj: &PyObjectRef, arg: Option<&PyObjectRef>) -> Option<usize> {
    match arg {
        None => Some(0),
        Some(a) => {
            let b = a.borrow();
            match &*b {
                PyObject::Str(name) => {
                    let name = name.clone();
                    drop(b);
                    let names = obj.borrow().get_attribute("_group_names").ok()?;
                    let names_b = names.borrow();
                    if let PyObject::Dict(d) = &*names_b {
                        d.get(&py_str(&name))
                            .ok()
                            .flatten()
                            .and_then(|v| v.as_i64())
                            .map(|i| i as usize)
                    } else {
                        None
                    }
                }
                _ => {
                    drop(b);
                    a.as_i64().map(|i| i as usize)
                }
            }
        }
    }
}

/// Build a re.Match object with group(), groups(), groupdict(), start(),
/// end(), span() methods — `caps` carries ALL capture groups (not just the
/// whole match), so `m.group(1)`/`m.group('name')` etc. work: previously
/// this only ever stored the whole-match text (`_groups` was hardcoded to
/// an empty tuple, `group()` ignored any index/name argument entirely and
/// always returned the whole match) — real trigger: `html.unescape`'s own
/// `_replace_charref(m)` calling `m.group(1)`, and more broadly
/// `html.parser`/`_markupbase`'s tokenizer, which relies on named/indexed
/// groups throughout (`tagfind_tolerant`, `attrfind_tolerant`, etc.).
/// Returns `py_none()` if the regex didn't match.
pub fn make_match_object(
    re: &fancy_regex::Regex,
    caps: Option<fancy_regex::Captures<'_>>,
) -> PyObjectRef {
    make_match_object_detailed(re, caps, "", "", 0, 0, 0)
}

pub fn make_match_object_detailed(
    re: &fancy_regex::Regex,
    caps: Option<fancy_regex::Captures<'_>>,
    original_string: &str,
    pattern: &str,
    flags: i32,
    pos: usize,
    endpos: usize,
) -> PyObjectRef {
    match caps {
        Some(caps) => {
            let whole = caps.get(0).unwrap();
            let start_pos = whole.start();
            let end_pos = whole.end();
            let text = whole.as_str().to_string();

            // Per-group text/start/end (index 0 = whole match, matching
            // `_starts`/`_ends`/`_groups_text`'s indexing below) plus a
            // name->index map for `capture_names()`'s named groups.
            let n_groups = caps.len();
            let mut groups_text: Vec<PyObjectRef> = Vec::with_capacity(n_groups);
            let mut starts: Vec<PyObjectRef> = Vec::with_capacity(n_groups);
            let mut ends: Vec<PyObjectRef> = Vec::with_capacity(n_groups);
            for i in 0..n_groups {
                match caps.get(i) {
                    Some(g) => {
                        groups_text.push(py_str(g.as_str()));
                        starts.push(py_int(g.start() as i64));
                        ends.push(py_int(g.end() as i64));
                    }
                    None => {
                        groups_text.push(py_none());
                        starts.push(py_int(-1));
                        ends.push(py_int(-1));
                    }
                }
            }
            let mut name_to_index = crate::object::PyDict::new();
            for (i, name) in re.capture_names().enumerate() {
                if let Some(name) = name {
                    let _ = name_to_index.set(py_str(name), py_int(i as i64));
                }
            }

            let mut type_dict = HashMap::new();

            // group([n_or_name, ...]) — with no args, the whole match; with
            // one arg, that group's text (`None` if the group didn't
            // participate, matching real `re.Match.group`); with multiple
            // args, a tuple of each. Raises IndexError for an out-of-range
            // index or unknown name, matching real CPython.
            type_dict.insert_str(
                "group",
                PyObjectRef::new(PyObject::BuiltinFunction {
                    name: "group".to_string(),
                    func: |args| {
                        let self_obj = &args[0];
                        let group_texts = self_obj
                            .borrow()
                            .get_attribute("_groups_text")
                            .unwrap_or_else(|_| py_tuple(vec![]));
                        let fetch_one = |arg: Option<&PyObjectRef>| -> PyResult<PyObjectRef> {
                            let idx = resolve_group_arg(self_obj, arg)
                                .ok_or_else(|| PyError::IndexError("no such group".to_string()))?;
                            if let PyObject::Tuple(items) = &*group_texts.borrow() {
                                items
                                    .get(idx)
                                    .cloned()
                                    .ok_or_else(|| PyError::IndexError("no such group".to_string()))
                            } else {
                                Err(PyError::IndexError("no such group".to_string()))
                            }
                        };
                        if args.len() <= 1 {
                            fetch_one(None)
                        } else if args.len() == 2 {
                            fetch_one(Some(&args[1]))
                        } else {
                            let results: PyResult<Vec<PyObjectRef>> =
                                args[1..].iter().map(|a| fetch_one(Some(a))).collect();
                            Ok(py_tuple(results?))
                        }
                    },
                }),
            );

            // groups(default=None) — tuple of ALL captured groups (1..N,
            // excluding the whole match at index 0), substituting `default`
            // for any group that didn't participate (None otherwise).
            type_dict.insert_str(
                "groups",
                PyObjectRef::new(PyObject::BuiltinFunction {
                    name: "groups".to_string(),
                    func: |args| {
                        let default = args.get(1).cloned().unwrap_or_else(py_none);
                        let group_texts = args[0]
                            .borrow()
                            .get_attribute("_groups_text")
                            .unwrap_or_else(|_| py_tuple(vec![]));
                        let result = if let PyObject::Tuple(items) = &*group_texts.borrow() {
                            let rest: Vec<PyObjectRef> = items
                                .iter()
                                .skip(1)
                                .map(|v| {
                                    if matches!(&*v.borrow(), PyObject::None) {
                                        default.clone()
                                    } else {
                                        v.clone()
                                    }
                                })
                                .collect();
                            Ok(py_tuple(rest))
                        } else {
                            Ok(py_tuple(vec![]))
                        };
                        result
                    },
                }),
            );

            // groupdict(default=None) — {name: value} for every NAMED group.
            type_dict.insert_str(
                "groupdict",
                PyObjectRef::new(PyObject::BuiltinFunction {
                    name: "groupdict".to_string(),
                    func: |args| {
                        let default = args.get(1).cloned().unwrap_or_else(py_none);
                        let names = args[0]
                            .borrow()
                            .get_attribute("_group_names")
                            .unwrap_or_else(|_| {
                                PyObjectRef::new(PyObject::Dict(Box::new(
                                    crate::object::PyDict::new(),
                                )))
                            });
                        let group_texts = args[0]
                            .borrow()
                            .get_attribute("_groups_text")
                            .unwrap_or_else(|_| py_tuple(vec![]));
                        let mut result = crate::object::PyDict::new();
                        if let (PyObject::Dict(names_d), PyObject::Tuple(items)) =
                            (&*names.borrow(), &*group_texts.borrow())
                        {
                            for (k, v) in names_d.iter() {
                                let idx = v.as_i64().unwrap_or(0) as usize;
                                let val = items.get(idx).cloned().unwrap_or_else(py_none);
                                let val = if matches!(&*val.borrow(), PyObject::None) {
                                    default.clone()
                                } else {
                                    val
                                };
                                let _ = result.set(k.clone(), val);
                            }
                        }
                        Ok(PyObjectRef::new(PyObject::Dict(Box::new(result))))
                    },
                }),
            );

            // start([n_or_name]) — start position of the whole match or a
            // specific group (-1 if that group didn't participate).
            type_dict.insert_str(
                "start",
                PyObjectRef::new(PyObject::BuiltinFunction {
                    name: "start".to_string(),
                    func: |args| {
                        let self_obj = &args[0];
                        let idx = resolve_group_arg(self_obj, args.get(1))
                            .ok_or_else(|| PyError::IndexError("no such group".to_string()))?;
                        let starts = self_obj
                            .borrow()
                            .get_attribute("_starts")
                            .unwrap_or_else(|_| py_tuple(vec![]));
                        let result = if let PyObject::Tuple(items) = &*starts.borrow() {
                            items
                                .get(idx)
                                .cloned()
                                .ok_or_else(|| PyError::IndexError("no such group".to_string()))
                        } else {
                            Ok(py_int(-1))
                        };
                        result
                    },
                }),
            );

            // end([n_or_name]) — end position of the whole match or a
            // specific group (-1 if that group didn't participate).
            type_dict.insert_str(
                "end",
                PyObjectRef::new(PyObject::BuiltinFunction {
                    name: "end".to_string(),
                    func: |args| {
                        let self_obj = &args[0];
                        let idx = resolve_group_arg(self_obj, args.get(1))
                            .ok_or_else(|| PyError::IndexError("no such group".to_string()))?;
                        let ends = self_obj
                            .borrow()
                            .get_attribute("_ends")
                            .unwrap_or_else(|_| py_tuple(vec![]));
                        let result = if let PyObject::Tuple(items) = &*ends.borrow() {
                            items
                                .get(idx)
                                .cloned()
                                .ok_or_else(|| PyError::IndexError("no such group".to_string()))
                        } else {
                            Ok(py_int(-1))
                        };
                        result
                    },
                }),
            );

            // span([n_or_name]) — (start, end) tuple, whole match or a group.
            type_dict.insert_str(
                "span",
                PyObjectRef::new(PyObject::BuiltinFunction {
                    name: "span".to_string(),
                    func: |args| {
                        let self_obj = &args[0];
                        let idx = resolve_group_arg(self_obj, args.get(1))
                            .ok_or_else(|| PyError::IndexError("no such group".to_string()))?;
                        let starts = self_obj
                            .borrow()
                            .get_attribute("_starts")
                            .unwrap_or_else(|_| py_tuple(vec![]));
                        let ends = self_obj
                            .borrow()
                            .get_attribute("_ends")
                            .unwrap_or_else(|_| py_tuple(vec![]));
                        let s = if let PyObject::Tuple(items) = &*starts.borrow() {
                            items.get(idx).cloned()
                        } else {
                            None
                        };
                        let e = if let PyObject::Tuple(items) = &*ends.borrow() {
                            items.get(idx).cloned()
                        } else {
                            None
                        };
                        match (s, e) {
                            (Some(s), Some(e)) => Ok(py_tuple(vec![s, e])),
                            _ => Err(PyError::IndexError("no such group".to_string())),
                        }
                    },
                }),
            );

            // __getitem__ — match[0] returns full match (== group(0)),
            // match[n_or_name] same as group(n_or_name) for n/name >= 1.
            type_dict.insert_str(
                "__getitem__",
                PyObjectRef::new(PyObject::BuiltinFunction {
                    name: "__getitem__".to_string(),
                    func: |args| {
                        if args.len() < 2 {
                            return Err(PyError::type_error("__getitem__ requires index"));
                        }
                        let self_obj = &args[0];
                        let idx = resolve_group_arg(self_obj, Some(&args[1]))
                            .ok_or_else(|| PyError::IndexError("no such group".to_string()))?;
                        let group_texts = self_obj
                            .borrow()
                            .get_attribute("_groups_text")
                            .unwrap_or_else(|_| py_tuple(vec![]));
                        let result = if let PyObject::Tuple(items) = &*group_texts.borrow() {
                            items
                                .get(idx)
                                .cloned()
                                .ok_or_else(|| PyError::IndexError("no such group".to_string()))
                        } else {
                            Err(PyError::IndexError("no such group".to_string()))
                        };
                        result
                    },
                }),
            );

            // __bool__ — always True for a successful match
            type_dict.insert_str(
                "__bool__",
                PyObjectRef::new(PyObject::BuiltinFunction {
                    name: "__bool__".to_string(),
                    func: |_| Ok(py_bool(true)),
                }),
            );

            type_dict.insert_str(
                "expand",
                PyObjectRef::new(PyObject::BuiltinFunction {
                    name: "expand".to_string(),
                    func: |args| {
                        if args.len() < 2 {
                            return Err(PyError::type_error("expand() takes at least 2 arguments"));
                        }
                        let self_obj = &args[0];
                        let template = args[1].str();
                        let translated = translate_python_replacement(&template);
                        let group_texts = self_obj
                            .borrow()
                            .get_attribute("_groups_text")
                            .unwrap_or_else(|_| py_tuple(vec![]));
                        let names = self_obj
                            .borrow()
                            .get_attribute("_group_names")
                            .unwrap_or_else(|_| PyObjectRef::new(PyObject::Dict(Box::new(crate::object::PyDict::new()))));
                        let mut result = String::new();
                        let chars: Vec<char> = translated.chars().collect();
                        let mut i = 0;
                        while i < chars.len() {
                            if chars[i] == '$' && i + 1 < chars.len() && chars[i + 1] == '{' {
                                let mut j = i + 2;
                                while j < chars.len() && chars[j] != '}' {
                                    j += 1;
                                }
                                let key: String = chars[i + 2..j].iter().collect();
                                let val = if let Ok(idx) = key.parse::<usize>() {
                                    if let PyObject::Tuple(items) = &*group_texts.borrow() {
                                        items.get(idx).cloned().unwrap_or_else(py_none)
                                    } else {
                                        py_none()
                                    }
                                } else {
                                    let names_b = names.borrow();
                                    if let PyObject::Dict(d) = &*names_b {
                                        if let Some(v) = d.get(&py_str(&key)).ok().flatten().and_then(|v| v.as_i64()) {
                                            if let PyObject::Tuple(items) = &*group_texts.borrow() {
                                                items.get(v as usize).cloned().unwrap_or_else(py_none)
                                            } else {
                                                py_none()
                                            }
                                        } else {
                                            py_none()
                                        }
                                    } else {
                                        py_none()
                                    }
                                };
                                if !matches!(&*val.borrow(), PyObject::None) {
                                    result.push_str(&val.str());
                                }
                                i = if j < chars.len() { j + 1 } else { j };
                            } else if chars[i] == '$' && i + 1 < chars.len() && chars[i + 1] == '$' {
                                result.push('$');
                                i += 2;
                            } else {
                                result.push(chars[i]);
                                i += 1;
                            }
                        }
                        Ok(py_str(&result))
                    },
                }),
            );

            type_dict.insert_str(
                "__repr__",
                PyObjectRef::new(PyObject::BuiltinFunction {
                    name: "__repr__".to_string(),
                    func: |args| {
                        let self_obj = &args[0];
                        let text = self_obj.borrow().get_attribute("_text").unwrap_or_else(|_| py_str("")).str();
                        let s = self_obj.borrow().get_attribute("_start").unwrap_or_else(|_| py_int(0)).str();
                        let e = self_obj.borrow().get_attribute("_end").unwrap_or_else(|_| py_int(0)).str();
                        Ok(py_str(&format!("<re.Match object; span=({}, {}), match={}>", s, e, text)))
                    },
                }),
            );

            let typ = PyObjectRef::new(PyObject::Type {
                name: "Match".to_string(),
                dict: Box::new(str_map_to_typedict(type_dict)),
                bases: vec![],
                mro: vec![],
            });

            let mut lastindex: Option<i64> = None;
            let mut lastgroup: Option<String> = None;
            for idx in (1..groups_text.len()).rev() {
                if !matches!(&*groups_text[idx].borrow(), PyObject::None) {
                    lastindex = Some(idx as i64);
                    for (k, v) in name_to_index.iter() {
                        if v.as_i64() == Some(idx as i64) {
                            lastgroup = Some(k.str());
                            break;
                        }
                    }
                    break;
                }
            }
            let regs_items: Vec<PyObjectRef> = starts.iter().zip(ends.iter()).map(|(s, e)| py_tuple(vec![s.clone(), e.clone()])).collect();
            let effective_pos = pos;
            let effective_endpos = if endpos == 0 { original_string.len() } else { endpos };
            let effective_string = if original_string.is_empty() { text.clone() } else { original_string.to_string() };
            let re_obj = PyObjectRef::new(PyObject::CompiledRegex {
                regex: Box::new(re.clone()),
                pattern: pattern.to_string(),
                flags,
            });

            let mut instance_dict = AttrMap::new();
            instance_dict.insert_str("_text", py_str(&text));
            instance_dict.insert_str("_start", py_int(start_pos as i64));
            instance_dict.insert_str("_end", py_int(end_pos as i64));
            // `_groups_text`/`_starts`/`_ends` are 0-indexed with index 0 =
            // the whole match (matching real `re.Match`'s own `group(0)`/
            // `[0]` convention) — `groups()` skips index 0 when building its
            // 1..N tuple.
            instance_dict.insert_str("_groups_text", py_tuple(groups_text.clone()));
            instance_dict.insert_str("_starts", py_tuple(starts.clone()));
            instance_dict.insert_str("_ends", py_tuple(ends.clone()));
            instance_dict.insert_str(
                "_group_names",
                PyObjectRef::new(PyObject::Dict(Box::new(name_to_index))),
            );
            instance_dict.insert_str("re", re_obj.clone());
            instance_dict.insert_str("string", py_str(&effective_string));
            instance_dict.insert_str("pos", py_int(effective_pos as i64));
            instance_dict.insert_str("endpos", py_int(effective_endpos as i64));
            instance_dict.insert_str("regs", py_tuple(regs_items));
            instance_dict.insert_str("lastindex", lastindex.map(py_int).unwrap_or_else(py_none));
            instance_dict.insert_str("lastgroup", lastgroup.map(|s| py_str(&s)).unwrap_or_else(py_none));
            instance_dict.insert_str("_re", re_obj.clone());

            PyObjectRef::new(PyObject::Instance {
                typ,
                dict: instance_dict,
            })
        }
        None => py_none(),
    }
}

