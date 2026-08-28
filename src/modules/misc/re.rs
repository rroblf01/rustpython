use crate::object::*;
use std::collections::HashMap;

/// Python's `re` treats a `{` that doesn't form a valid `{n}`/`{n,}`/`{n,m}`
/// counted-repetition quantifier as a literal character; Rust's `regex`
/// crate instead rejects it as a parse error ("repetition operator missing
/// expression"). Real-world patterns lean on this leniency constantly
/// (e.g. Django's template-tag detector `{%.*?%}`), so translate patterns
/// through this before compiling rather than surfacing the raw Rust error.
fn escape_loose_braces(pattern: &str) -> String {
    let chars: Vec<char> = pattern.chars().collect();
    let mut result = String::with_capacity(pattern.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\\' && i + 1 < chars.len() {
            result.push(c);
            result.push(chars[i + 1]);
            i += 2;
            continue;
        }
        if c == '{' {
            let mut j = i + 1;
            let mut saw_digit = false;
            while j < chars.len() && chars[j].is_ascii_digit() {
                j += 1;
                saw_digit = true;
            }
            if j < chars.len() && chars[j] == ',' {
                j += 1;
                while j < chars.len() && chars[j].is_ascii_digit() {
                    j += 1;
                }
            }
            if saw_digit && j < chars.len() && chars[j] == '}' {
                result.extend(&chars[i..=j]);
                i = j + 1;
            } else {
                result.push_str("\\{");
                i += 1;
            }
            continue;
        }
        result.push(c);
        i += 1;
    }
    result
}

/// Two related Python-`re`-vs-Rust-`regex` character-class gaps in one
/// pass, both hit by real code (CPython's own `email.utils.specialsre`,
/// `r'[][\\()<>@,:;".]'`):
///
/// 1. A `]` right after the opening `[` (or `[^`) of a class is a literal
///    `]`, not the closing bracket — Rust's `regex` crate actually *does*
///    already support this one natively (confirmed: `[]]`/`[]x]` compile
///    fine as-is) — no translation needed for this part by itself.
/// 2. A bare `[` appearing *inside* an already-open class (a plain literal
///    character there in Python/POSIX/PCRE — classes don't nest) is
///    mistaken by Rust's `regex` crate for the start of a *nested* class
///    it doesn't support, failing with "unclosed character class" the
///    moment the class also contains an unescaped `]` later (confirmed:
///    `[]x]` alone is fine, but `[][x]` and `[][\\()<>@,:;".]` both fail;
///    `[]\[]`, with the inner `[` pre-escaped, works). This is the part
///    that actually needs translating — every bare `[` found while already
///    inside a class gets escaped to `\[`.
///
/// Both are handled by the same single-pass `in_class` scan below (the
/// leading-`]` case doesn't need output changes, just correct state
/// tracking so the following bare-`[` fix doesn't misfire on it). The same
/// pass also translates octal character escapes (`\NNN`) to `\x{...}` when
/// inside a class — see the comment at that branch for why it's scoped to
/// in-class only.
fn escape_leading_bracket_in_class(pattern: &str) -> String {
    let chars: Vec<char> = pattern.chars().collect();
    let mut result = String::with_capacity(pattern.len());
    let mut i = 0;
    let mut in_class = false;
    while i < chars.len() {
        let c = chars[i];
        if c == '\\' && i + 1 < chars.len() {
            // Python's `re` accepts `\NNN` (1-3 octal digits) as an octal
            // character escape (real code: CPython's own `email.header`,
            // `r'[\041-\176]+:$'`, an ASCII-printable-range class). Rust's
            // `regex` crate has no octal-escape syntax and reads a
            // backslash-digit sequence as a *backreference* attempt
            // instead — which it doesn't support at all — rejecting it
            // outright. Only translate this inside a character class,
            // where a backreference could never be valid syntax in any
            // regex flavor anyway (so there's no ambiguity to worry about,
            // unlike outside a class where `\1` etc. legitimately mean
            // "backreference to group 1" in real patterns elsewhere).
            if in_class && chars[i + 1].is_digit(8) {
                let mut j = i + 1;
                let mut value: u32 = 0;
                let mut digits = 0;
                while j < chars.len() && digits < 3 && chars[j].is_digit(8) {
                    value = value * 8 + chars[j].to_digit(8).unwrap();
                    j += 1;
                    digits += 1;
                }
                result.push_str(&format!("\\x{{{:x}}}", value));
                i = j;
                continue;
            }
            result.push(c);
            result.push(chars[i + 1]);
            i += 2;
            continue;
        }
        if !in_class && c == '[' {
            result.push(c);
            i += 1;
            in_class = true;
            if i < chars.len() && chars[i] == '^' {
                result.push('^');
                i += 1;
            }
            // A `]` right here (the very first character of the class,
            // after an optional `^`) is a literal `]`, not the closing
            // bracket — Rust's `regex` crate needs it spelled `\]` to
            // agree; every subsequent `[`/`]` until the *real* close is
            // handled by the `in_class` tracking below instead of this
            // one-shot check, so a second literal `[` right after (like
            // `r'[][...]'`, `]` then `[` both literal) is never mistaken
            // for the start of a nested class.
            if i < chars.len() && chars[i] == ']' {
                result.push('\\');
                result.push(']');
                i += 1;
            }
            continue;
        }
        if in_class && c == ']' {
            in_class = false;
            result.push(c);
            i += 1;
            continue;
        }
        if in_class && c == '[' {
            // A bare `[` here is just a literal character in Python/POSIX
            // (classes don't nest) — Rust's `regex` crate reads it as
            // attempting a nested class instead, so escape it.
            result.push('\\');
            result.push('[');
            i += 1;
            continue;
        }
        result.push(c);
        i += 1;
    }
    result
}

/// Python's `re.sub`/`Pattern.sub` replacement strings reference capture
/// groups as `\1`, `\g<1>`, `\g<name>` — the `regex`/`fancy_regex` crates'
/// `Replacer` impl for `&str` instead uses Perl/sed-style `$1`/`${1}`/`${name}`
/// and treats a literal `$` specially. Translate before calling
/// `replace_all`/`replace`, or every `\N`-backreference replacement (an
/// extremely common idiom — e.g. Django's own `camel_case_to_spaces`:
/// `re_camel_case.sub(r" \1", value)`) silently emits the backreference
/// syntax itself instead of the captured text.
pub(crate) fn translate_python_replacement(repl: &str) -> String {
    let chars: Vec<char> = repl.chars().collect();
    let mut out = String::with_capacity(repl.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '$' {
            out.push_str("$$");
            i += 1;
        } else if c == '\\' && i + 1 < chars.len() {
            let next = chars[i + 1];
            if next.is_ascii_digit() {
                let mut j = i + 1;
                while j < chars.len() && chars[j].is_ascii_digit() {
                    j += 1;
                }
                out.push_str("${");
                out.extend(&chars[i + 1..j]);
                out.push('}');
                i = j;
            } else if next == 'g' && chars.get(i + 2) == Some(&'<') {
                let mut j = i + 3;
                while j < chars.len() && chars[j] != '>' {
                    j += 1;
                }
                out.push_str("${");
                out.extend(&chars[i + 3..j]);
                out.push('}');
                i = if j < chars.len() { j + 1 } else { j };
            } else if next == '\\' {
                out.push('\\');
                i += 2;
            } else {
                out.push(next);
                i += 2;
            }
        } else {
            out.push(c);
            i += 1;
        }
    }
    out
}

fn compile_python_regex(pattern: &str) -> Result<fancy_regex::Regex, fancy_regex::Error> {
    compile_python_regex_flags(pattern, 0)
}

/// Same as `compile_python_regex`, but applies `re.compile(pattern, flags)`'s
/// `flags` argument — previously accepted and stored on `CompiledRegex` (for
/// `.flags` attribute introspection) but never actually influenced
/// compilation at all, so e.g. `re.IGNORECASE`/`re.VERBOSE`/`re.MULTILINE`/
/// `re.DOTALL` were all silently no-ops. Real trigger: `html.parser`'s own
/// `locatetagend = re.compile(r"""...""", re.VERBOSE)` — a pattern that's
/// entirely unparseable as-is without VERBOSE's whitespace/comment
/// stripping (every space and `# comment` in the triple-quoted pattern is
/// otherwise literal regex syntax). Translated to the regex engine's own
/// inline flag group (`(?ismx)...`) prepended to the pattern — `regex`/
/// `fancy_regex`'s own flag semantics for `i`/`s`/`m`/`x` match Python's
/// IGNORECASE/DOTALL/MULTILINE/VERBOSE closely enough for real-world use.
fn compile_python_regex_flags(
    pattern: &str,
    flags: i32,
) -> Result<fancy_regex::Regex, fancy_regex::Error> {
    let pattern = escape_loose_braces(pattern);
    let pattern = escape_leading_bracket_in_class(&pattern);
    let mut inline = String::new();
    if flags & 2 != 0 {
        inline.push('i');
    } // IGNORECASE
    if flags & 16 != 0 {
        inline.push('s');
    } // DOTALL
    if flags & 8 != 0 {
        inline.push('m');
    } // MULTILINE
    if flags & 64 != 0 {
        inline.push('x');
    } // VERBOSE
    let pattern = if inline.is_empty() {
        pattern
    } else {
        format!("(?{}){}", inline, pattern)
    };
    fancy_regex::Regex::new(&pattern)
}

/// Resolve a `group()`/`start()`/`end()` group argument (an int index, OR a
/// string/name — real `re.Match` accepts both) against the match's stored
/// `_group_names` dict (name -> 1-based index), returning a 0-based index
/// into `_starts`/`_ends`/`_groups_text` (0 = whole match). Returns `None`
/// for a name that doesn't exist (caller raises `IndexError`, matching
/// real CPython's `no such group`).
fn resolve_group_arg(obj: &PyObjectRef, arg: Option<&PyObjectRef>) -> Option<usize> {
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
pub(crate) fn make_match_object(
    re: &fancy_regex::Regex,
    caps: Option<fancy_regex::Captures<'_>>,
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

            let typ = PyObjectRef::new(PyObject::Type {
                name: "Match".to_string(),
                dict: Box::new(str_map_to_typedict(type_dict)),
                bases: vec![],
                mro: vec![],
            });

            let mut instance_dict = AttrMap::new();
            instance_dict.insert_str("_text", py_str(&text));
            instance_dict.insert_str("_start", py_int(start_pos as i64));
            instance_dict.insert_str("_end", py_int(end_pos as i64));
            // `_groups_text`/`_starts`/`_ends` are 0-indexed with index 0 =
            // the whole match (matching real `re.Match`'s own `group(0)`/
            // `[0]` convention) — `groups()` skips index 0 when building its
            // 1..N tuple.
            instance_dict.insert_str("_groups_text", py_tuple(groups_text));
            instance_dict.insert_str("_starts", py_tuple(starts));
            instance_dict.insert_str("_ends", py_tuple(ends));
            instance_dict.insert_str(
                "_group_names",
                PyObjectRef::new(PyObject::Dict(Box::new(name_to_index))),
            );

            PyObjectRef::new(PyObject::Instance {
                typ,
                dict: instance_dict,
            })
        }
        None => py_none(),
    }
}

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
                Ok(make_match_object(&re, caps))
            }
            Err(e) => Err(PyError::ValueError(format!("invalid regex: {}", e))),
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
                Ok(make_match_object(&re, result))
            }
            Err(e) => Err(PyError::ValueError(format!("invalid regex: {}", e))),
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
                Ok(make_match_object(&re, caps))
            }
            Err(e) => Err(PyError::ValueError(format!("invalid regex: {}", e))),
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
            Err(e) => Err(PyError::ValueError(format!("invalid regex: {}", e))),
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
                        let match_obj = make_match_object(&re, Some(caps));
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
            Err(e) => Err(PyError::ValueError(format!("invalid regex: {}", e))),
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
                Err(e) => return Err(PyError::ValueError(format!("invalid regex: {}", e))),
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
            Err(e) => Err(PyError::ValueError(format!("invalid regex: {}", e))),
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
                    .map(|c| make_match_object(&re, Some(c)))
                    .collect();
                // Return a list that can be iterated over
                Ok(py_list(matches))
            }
            Err(e) => Err(PyError::ValueError(format!("invalid regex: {}", e))),
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
    d.insert_str("I", py_int(2));
    d.insert_str("A", py_int(256));
    d.insert_str("S", py_int(16));
    d.insert_str("M", py_int(8));
    d.insert_str("X", py_int(64));

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

    d
}
