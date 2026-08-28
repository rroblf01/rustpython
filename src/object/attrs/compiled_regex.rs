// Auto-extracted from src/object/attrs/mod.rs lines 6940-7173
use crate::object::*;
use super::*;
use crate::interner;

pub(crate) fn get(o: &PyObject, name: &str) -> PyResult<PyObjectRef> {
    match o {
            PyObject::CompiledRegex {
                regex,
                pattern,
                flags,
            } => {
                let re = (**regex).clone();
                let pat = pattern.clone();
                let fl = *flags;
                match name {
                    "pattern" => Ok(py_str(&pat)),
                    "flags" => Ok(py_int(fl as i64)),
                    // `match`/`search`/`fullmatch` used to return a bare
                    // `(start, end, matched_text)` tuple instead of a real
                    // `Match` object — no `.group(n)`/`.groups()`/etc. at
                    // all, so any code capturing groups via `Pattern.
                    // match(...).group(1)` (extremely common — this is
                    // exactly how `html.parser`/`_markupbase`'s tokenizer
                    // works throughout) got `AttributeError: 'tuple' object
                    // has no attribute 'group'`. Delegates to
                    // `crate::modules::make_match_object` — the same
                    // capture-group-aware Match object the free `re.match`/
                    // `re.search`/etc. functions build (see that function's
                    // own doc comment for the fuller history).
                    // Both accept an optional `pos` 2nd argument (`Pattern.
                    // match(string, pos)`/`Pattern.search(string, pos)` —
                    // start searching from `pos` rather than the start of
                    // the string, WITHOUT losing context before `pos` for
                    // lookbehind-style constructs (unlike naively slicing
                    // the string at `pos` and matching against that).
                    // `_markupbase`/`html.parser`'s tokenizer calls this
                    // constantly (`locatetagend.match(rawdata, i+1)`) to
                    // resume scanning from wherever the last token ended.
                    "match" => Ok(PyObjectRef::imm(PyObject::Closure(Rc::new(
                        move |args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                            if args.len() < 1 {
                                return Err(PyError::type_error(
                                    "match() takes at least 1 argument",
                                ));
                            }
                            let string = args[0].str();
                            let pos =
                                args.get(1).and_then(|a| a.as_i64()).unwrap_or(0).max(0) as usize;
                            let caps = re
                                .captures_from_pos(&string, pos.min(string.len()))
                                .unwrap_or(None);
                            let result = match caps {
                                Some(c) if c.get(0).map(|m| m.start()) == Some(pos) => Some(c),
                                _ => None,
                            };
                            Ok(crate::modules::make_match_object(&re, result))
                        },
                    )))),
                    "search" => Ok(PyObjectRef::imm(PyObject::Closure(Rc::new(
                        move |args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                            if args.len() < 1 {
                                return Err(PyError::type_error(
                                    "search() takes at least 1 argument",
                                ));
                            }
                            let string = args[0].str();
                            let pos =
                                args.get(1).and_then(|a| a.as_i64()).unwrap_or(0).max(0) as usize;
                            let caps = re
                                .captures_from_pos(&string, pos.min(string.len()))
                                .unwrap_or(None);
                            Ok(crate::modules::make_match_object(&re, caps))
                        },
                    )))),
                    "findall" => Ok(PyObjectRef::imm(PyObject::Closure(Rc::new(
                        move |args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                            if args.len() < 1 {
                                return Err(PyError::type_error(
                                    "findall() takes at least 1 argument",
                                ));
                            }
                            let string = args[0].str();
                            let results: Vec<PyObjectRef> = re
                                .find_iter(&string)
                                .filter_map(|r| r.ok())
                                .map(|m| py_str(m.as_str()))
                                .collect();
                            Ok(py_list(results))
                        },
                    )))),
                    "finditer" => Ok(PyObjectRef::imm(PyObject::Closure(Rc::new(
                        move |args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                            if args.len() < 1 {
                                return Err(PyError::type_error(
                                    "finditer() takes at least 1 argument",
                                ));
                            }
                            let string = args[0].str();
                            let matches: Vec<PyObjectRef> = re
                                .captures_iter(&string)
                                .filter_map(|r| r.ok())
                                .map(|c| crate::modules::make_match_object(&re, Some(c)))
                                .collect();
                            Ok(py_list(matches))
                        },
                    )))),
                    // Real `re.Pattern.sub` accepts either a string template
                    // OR a callable — see the free `re.sub` function's own
                    // doc comment (`modules/misc.rs`) for the fuller
                    // history; this mirrors that fix (and adds real `count`
                    // support) for the compiled-`Pattern` method form.
                    "sub" => Ok(PyObjectRef::imm(PyObject::Closure(Rc::new(
                        move |args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "sub() takes at least 2 arguments",
                                ));
                            }
                            let is_callable_repl = !matches!(&*args[0].borrow(), PyObject::Str(_));
                            let repl_template = if is_callable_repl {
                                String::new()
                            } else {
                                crate::modules::translate_python_replacement(&args[0].str())
                            };
                            let string = args[1].str();
                            let count = if args.len() > 2 {
                                args[2].as_i64().unwrap_or(0)
                            } else {
                                0
                            };
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
                                    let match_obj =
                                        crate::modules::make_match_object(&re, Some(caps));
                                    let replaced =
                                        call_bound_method(args[0].clone(), match_obj, vec![])?;
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
                        },
                    )))),
                    "split" => Ok(PyObjectRef::imm(PyObject::Closure(Rc::new(
                        move |args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                            if args.len() < 1 {
                                return Err(PyError::type_error(
                                    "split() takes at least 1 argument",
                                ));
                            }
                            let string = args[0].str();
                            let maxsplit = {
                                let has_kwargs = args.last().map_or(false, |a| matches!(&*a.borrow(), PyObject::Dict(_)));
                                if has_kwargs {
                                    if let PyObject::Dict(d) = &*args.last().unwrap().borrow() {
                                        d.get(&py_str("maxsplit")).ok().flatten().and_then(|v| v.as_i64()).unwrap_or(0) as usize
                                    } else { 0 }
                                } else if args.len() > 1 {
                                    if matches!(&*args[1].borrow(), PyObject::Dict(_)) { 0 } else { args[1].as_i64().unwrap_or(0) as usize }
                                } else { 0 }
                            };
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
                        },
                    )))),
                    "fullmatch" => Ok(PyObjectRef::imm(PyObject::Closure(Rc::new(
                        move |args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                            if args.len() < 1 {
                                return Err(PyError::type_error(
                                    "fullmatch() takes at least 1 argument",
                                ));
                            }
                            let string = args[0].str();
                            let caps = re.captures(&string).unwrap_or(None).filter(|c| {
                                c.get(0)
                                    .map(|m| m.start() == 0 && m.end() == string.len())
                                    .unwrap_or(false)
                            });
                            Ok(crate::modules::make_match_object(&re, caps))
                        },
                    )))),
                    "subn" => Ok(PyObjectRef::imm(PyObject::Closure(Rc::new(
                        move |args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "subn() takes at least 2 arguments",
                                ));
                            }
                            let is_callable_repl = !matches!(&*args[0].borrow(), PyObject::Str(_));
                            let repl_template = if is_callable_repl {
                                String::new()
                            } else {
                                crate::modules::translate_python_replacement(&args[0].str())
                            };
                            let string = args[1].str();
                            let count = if args.len() > 2 {
                                args[2].as_i64().unwrap_or(0)
                            } else {
                                0
                            };
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
                                    let match_obj =
                                        crate::modules::make_match_object(&re, Some(caps));
                                    let replaced =
                                        call_bound_method(args[0].clone(), match_obj, vec![])?;
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
                        },
                    )))),
                    "scanner" => Ok(PyObjectRef::imm(PyObject::Closure(Rc::new(
                        move |args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                            if args.is_empty() {
                                return Err(PyError::type_error("scanner() missing string argument"));
                            }
                            let string = args[0].str();
                            let pos = args.get(1).and_then(|a| a.as_i64()).unwrap_or(0).max(0) as usize;
                            let endpos = args.get(2).and_then(|a| a.as_i64()).unwrap_or(string.len() as i64).max(0) as usize;
                            let s = string.clone();
                            let p = pat.clone();
                            let fl = fl;
                            // Build scanner type with search/match
                            let mut scan_dict = std::collections::HashMap::new();
                            scan_dict.insert(
                                "search".to_string(),
                                PyObjectRef::new(PyObject::BuiltinFunction {
                                    name: "search".to_string(),
                                    func: |a| {
                                        if a.is_empty() {
                                            return Err(PyError::type_error("search() missing self"));
                                        }
                                        let self_obj = &a[0];
                                        let string_v = self_obj.borrow().get_attribute("_string").unwrap_or_else(|_| py_str(""));
                                        let s = string_v.str();
                                        let pos_v = self_obj.borrow().get_attribute("_pos").ok().and_then(|v| v.as_i64()).unwrap_or(0) as usize;
                                        let endpos_v = self_obj.borrow().get_attribute("_endpos").ok().and_then(|v| v.as_i64()).unwrap_or(s.len() as i64) as usize;
                                        let re_v = self_obj.borrow().get_attribute("_re").unwrap_or_else(|_| py_none());
                                        let re_clone = if let PyObject::CompiledRegex{ regex, .. } = &*re_v.borrow() {
                                            (**regex).clone()
                                        } else {
                                            return Err(PyError::runtime_error("invalid scanner re"));
                                        };
                                        let endpos_v = endpos_v.min(s.len());
                                        let pos_v = pos_v.min(s.len());
                                        let caps = re_clone.captures_from_pos(&s, pos_v).unwrap_or(None);
                                        if let Some(caps) = caps {
                                            if let Some(m) = caps.get(0) {
                                                if m.end() <= endpos_v {
                                                    let new_pos = m.end();
                                                    if let PyObject::Instance{ dict, .. } = &mut *self_obj.borrow_mut() {
                                                        dict.insert_str("_pos", py_int(new_pos as i64));
                                                    }
                                                    return Ok(crate::modules::make_match_object(&re_clone, Some(caps)));
                                                }
                                            }
                                        }
                                        Ok(py_none())
                                    },
                                }),
                            );
                            scan_dict.insert(
                                "match".to_string(),
                                PyObjectRef::new(PyObject::BuiltinFunction {
                                    name: "match".to_string(),
                                    func: |a| {
                                        if a.is_empty() {
                                            return Err(PyError::type_error("match() missing self"));
                                        }
                                        let self_obj = &a[0];
                                        let string_v = self_obj.borrow().get_attribute("_string").unwrap_or_else(|_| py_str(""));
                                        let s = string_v.str();
                                        let pos_v = self_obj.borrow().get_attribute("_pos").ok().and_then(|v| v.as_i64()).unwrap_or(0) as usize;
                                        let endpos_v = self_obj.borrow().get_attribute("_endpos").ok().and_then(|v| v.as_i64()).unwrap_or(s.len() as i64) as usize;
                                        let re_v = self_obj.borrow().get_attribute("_re").unwrap_or_else(|_| py_none());
                                        let re_clone = if let PyObject::CompiledRegex{ regex, .. } = &*re_v.borrow() {
                                            (**regex).clone()
                                        } else {
                                            return Err(PyError::runtime_error("invalid scanner re"));
                                        };
                                        let caps = re_clone.captures_from_pos(&s, pos_v).unwrap_or(None);
                                        if let Some(caps) = caps {
                                            if let Some(m) = caps.get(0) {
                                                if m.start() == pos_v && m.end() <= endpos_v {
                                                    let new_pos = m.end();
                                                    if let PyObject::Instance{ dict, .. } = &mut *self_obj.borrow_mut() {
                                                        dict.insert_str("_pos", py_int(new_pos as i64));
                                                    }
                                                    return Ok(crate::modules::make_match_object(&re_clone, Some(caps)));
                                                }
                                            }
                                        }
                                        Ok(py_none())
                                    },
                                }),
                            );
                            let scan_type = PyObjectRef::new(PyObject::Type{
                                name: "SRE_Scanner".to_string(),
                                dict: Box::new(str_map_to_typedict(scan_dict)),
                                bases: vec![],
                                mro: vec![],
                            });
                            if let PyObject::Type{ mro, .. } = &mut *scan_type.borrow_mut(){
                                *mro = vec![scan_type.clone()];
                            }
                            let re_obj = PyObjectRef::new(PyObject::CompiledRegex{
                                regex: Box::new(re.clone()),
                                pattern: p.clone(),
                                flags: fl,
                            });
                            let mut inst_dict = AttrMap::new();
                            inst_dict.insert_str("_re", re_obj.clone());
                            inst_dict.insert_str("_string", py_str(&s));
                            inst_dict.insert_str("_pos", py_int(pos as i64));
                            inst_dict.insert_str("_endpos", py_int(endpos as i64));
                            inst_dict.insert_str("pattern", re_obj.clone());
                            Ok(PyObjectRef::new(PyObject::Instance{ typ: scan_type, dict: inst_dict }))
                        },
                    )))),
                    _ => Err(PyError::attribute_error(format!(
                        "'re.Pattern' object has no attribute '{}'",
                        name
                    ))),
                }
            }
        _ => unreachable!("{} called with non-{} object", stringify!(get), o.type_name()),
    }
}
