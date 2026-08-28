// Auto-extracted from src/object/attrs/mod.rs lines 5014-5779
use crate::object::*;
use super::*;
use crate::interner;
use unicode_general_category::{get_general_category, GeneralCategory};

pub(crate) fn get(o: &PyObject, name: &str) -> PyResult<PyObjectRef> {
    match o {
            PyObject::Str(_s) => {
                match name {
                    "replace" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "replace".to_string(),
                        func: |args| {
                            if args.len() < 3 {
                                return Err(PyError::type_error(
                                    "replace() takes at least 2 arguments",
                                ));
                            }
                            let s = args[0].str();
                            let old = args[1].str();
                            let new = args[2].str();
                            let max: i64 = {
                                if args.len() > 3 {
                                    // check for kwargs dict containing "count"
                                    if let PyObject::Dict(d) = &*args[args.len()-1].borrow() {
                                        if let Some(v) = d.get(&py_str("count")).ok().flatten() {
                                            v.as_i64().unwrap_or(-1)
                                        } else if args.len() == 4 {
                                            -1
                                        } else {
                                            args[3].as_i64().unwrap_or(-1)
                                        }
                                    } else {
                                        args[3].as_i64().unwrap_or(-1)
                                    }
                                } else { -1 }
                            };
                            if max == 0 { return Ok(py_str(&s)); }
                            let result = if old.is_empty() {
                                if max < 0 {
                                    let mut out = String::with_capacity(s.len() + (s.chars().count()+1)*new.len());
                                    out.push_str(&new);
                                    for ch in s.chars() { out.push(ch); out.push_str(&new); }
                                    out
                                } else {
                                    let m = max as usize;
                                    let total = std::cmp::min(m, s.chars().count()+1);
                                    let mut out = String::new();
                                    let mut inserted = 0;
                                    if total > 0 { out.push_str(&new); inserted+=1; }
                                    for ch in s.chars() {
                                        out.push(ch);
                                        if inserted < total { out.push_str(&new); inserted+=1; }
                                    }
                                    out
                                }
                            } else if max < 0 { s.replace(&old, &new) } else { s.replacen(&old, &new, max as usize) };
                            Ok(py_str(&result))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "isdecimal" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "isdecimal".to_string(),
                        func: |a| {
                            if a.len() != 1 {
                                return Err(PyError::type_error(
                                    "isdecimal() takes no arguments (1 given)",
                                ));
                            }

                            let s = a[0].str();
                            Ok(py_bool(
                                !s.is_empty()
                                    && s.chars().all(|c| get_general_category(c) == GeneralCategory::DecimalNumber),
                            ))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "isnumeric" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "isnumeric".to_string(),
                        func: |a| {
                            if a.len() != 1 {
                                return Err(PyError::type_error(
                                    "isnumeric() takes no arguments (1 given)",
                                ));
                            }

                            let s = a[0].str();
                            Ok(py_bool(
                                !s.is_empty()
                                    && s.chars().all(|c| c.is_numeric()),
                            ))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "isascii" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "isascii".to_string(),
                        func: |a| {
                            if a.len() != 1 {
                                return Err(PyError::type_error(
                                    "isascii() takes no arguments (1 given)",
                                ));
                            }
                            Ok(py_bool(a[0].str().is_ascii()))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "isprintable" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "isprintable".to_string(),
                        func: |a| {
                            if a.len() != 1 {
                                return Err(PyError::type_error(
                                    "isprintable() takes no arguments (1 given)",
                                ));
                            }

                            if a.len() != 1 {
                                return Err(PyError::type_error(
                                    "isascii() takes no arguments (1 given)",
                                ));
                            }

                            let s = a[0].str();
                            Ok(py_bool(s.chars().all(|c| {
                                let cat = get_general_category(c);
                                !c.is_control() && !matches!(cat, GeneralCategory::Format | GeneralCategory::Surrogate | GeneralCategory::PrivateUse | GeneralCategory::Unassigned | GeneralCategory::LineSeparator | GeneralCategory::ParagraphSeparator)
                            })))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "casefold" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "casefold".to_string(),
                        func: |a| {
                            if a.len() != 1 {
                                return Err(PyError::type_error(
                                    "casefold() takes no arguments (1 given)",
                                ));
                            }
                            Ok(py_str(&casefold_with_sigma(&a[0].str())))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "isdigit" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "isdigit".to_string(),
                        func: |a| {
                            if a.len() != 1 {
                                return Err(PyError::type_error(
                                    "isdigit() takes no arguments (1 given)",
                                ));
                            }
                            let s = a[0].str();
                            Ok(py_bool(!s.is_empty() && s.chars().all(|c| {
                                if get_general_category(c) == GeneralCategory::DecimalNumber { true }
                                else if matches!(c as u32, 0xB2..=0xB3 | 0xB9 | 0x1369..=0x1371 | 0x19DA | 0x2070 | 0x2074..=0x2079 | 0x2080..=0x2089 | 0x2460..=0x2468 | 0x2474..=0x247C | 0x2488..=0x2490 | 0x24EA | 0x24F5..=0x24FD | 0x24FF | 0x2776..=0x277E | 0x2780..=0x2788 | 0x278A..=0x2792 | 0x10A40..=0x10A43 | 0x10E60..=0x10E68 | 0x11052..=0x1105A | 0x1F100..=0x1F10A) { true }
                                else { false }
                            })))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "isalpha" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "isalpha".to_string(),
                        func: |a| {
                            if a.len() != 1 {
                                return Err(PyError::type_error(
                                    "isalpha() takes no arguments (1 given)",
                                ));
                            }
                            let s = a[0].str();
                            Ok(py_bool(!s.is_empty() && s.chars().all(|c| c.is_alphabetic())))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "isalnum" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "isalnum".to_string(),
                        func: |a| {
                            if a.len() != 1 {
                                return Err(PyError::type_error(
                                    "isalnum() takes no arguments (1 given)",
                                ));
                            }

                            let s = a[0].str();
                            Ok(py_bool(!s.is_empty() && s.chars().all(|c| c.is_alphanumeric())))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "isspace" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "isspace".to_string(),
                        func: |a| {
                            if a.len() != 1 {
                                return Err(PyError::type_error(
                                    "isspace() takes no arguments (1 given)",
                                ));
                            }
                            let s = a[0].str();
                            Ok(py_bool(!s.is_empty() && s.chars().all(|c| c.is_whitespace())))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "islower" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "islower".to_string(),
                        func: |a| {
                            if a.len() != 1 {
                                return Err(PyError::type_error(
                                    "islower() takes no arguments (1 given)",
                                ));
                            }
                            let s = a[0].str();
                            let mut has_cased = false;
                            let mut ok = true;
                            for c in s.chars() { if c.is_lowercase() { has_cased = true; } else if c.is_uppercase() { ok = false; break; } }
                            Ok(py_bool(has_cased && ok))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "isupper" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "isupper".to_string(),
                        func: |a| {
                            if a.len() != 1 {
                                return Err(PyError::type_error(
                                    "isupper() takes no arguments (1 given)",
                                ));
                            }
                            let s = a[0].str();
                            let mut has_cased = false;
                            let mut ok = true;
                            for c in s.chars() { if c.is_uppercase() { has_cased = true; } else if c.is_lowercase() { ok = false; break; } }
                            Ok(py_bool(has_cased && ok))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "istitle" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "istitle".to_string(),
                        func: |a| {
                            if a.len() != 1 {
                                return Err(PyError::type_error(
                                    "istitle() takes no arguments (1 given)",
                                ));
                            }

                            if a.len() != 1 {
                                return Err(PyError::type_error(
                                    "isupper() takes no arguments (1 given)",
                                ));
                            }

                            let s = a[0].str();
                            if s.is_empty() { return Ok(py_bool(false)); }
                            let mut prev_is_cased = false;
                            let mut has_cased = false;
                            let mut is_title = true;
                            for c in s.chars() {
                                let cat = get_general_category(c);
                                let is_upper = c.is_uppercase() || cat == GeneralCategory::TitlecaseLetter;
                                let is_lower = c.is_lowercase();
                                if is_upper {
                                    if prev_is_cased { is_title = false; break; }
                                    prev_is_cased = true;
                                    has_cased = true;
                                } else if is_lower {
                                    if !prev_is_cased { is_title = false; break; }
                                    prev_is_cased = true;
                                    has_cased = true;
                                } else if cat == GeneralCategory::UppercaseLetter || cat == GeneralCategory::LowercaseLetter || cat == GeneralCategory::TitlecaseLetter {
                                    // other cased? treat as cased but not upper/lower? keep prev
                                    prev_is_cased = true;
                                    has_cased = true;
                                } else if c.is_alphabetic() {
                                    prev_is_cased = true;
                                } else {
                                    prev_is_cased = false;
                                }
                            }
                            Ok(py_bool(is_title && has_cased))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "title" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "title".to_string(),
                        func: |a| {
                            if a.len() != 1 {
                                return Err(PyError::type_error(
                                    "title() takes no arguments (1 given)",
                                ));
                            }

                            let s = a[0].str();
                            let chars: Vec<char> = s.chars().collect();
                            let mut result = String::with_capacity(s.len());
                            let mut prev_is_cased = false;
                            for (i, &c) in chars.iter().enumerate() {
                                if is_cased(c) {
                                    if !prev_is_cased {
                                        let up: Vec<char> = if c == '\u{1FA1}' { vec!['\u{1FA9}'] } else { c.to_uppercase().collect() };
                                        if let Some(first) = up.first() {
                                            result.push(*first);
                                            for rest in up.iter().skip(1) {
                                                result.extend(rest.to_lowercase());
                                            }
                                        }
                                    } else {
                                        for lc in c.to_lowercase() {
                                            if lc == '\u{03C3}' {
                                                if is_final_sigma(&chars, i) {
                                                    result.push('\u{03C2}');
                                                } else {
                                                    result.push('\u{03C3}');
                                                }
                                            } else {
                                                result.push(lc);
                                            }
                                        }
                                    }
                                    prev_is_cased = true;
                                } else if is_case_ignorable(c) {
                                    result.push(c);
                                } else {
                                    result.push(c);
                                    prev_is_cased = false;
                                }
                            }
                            Ok(py_str(&result))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "capitalize" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "capitalize".to_string(),
                        func: |a| {
                            if a.len() != 1 {
                                return Err(PyError::type_error(
                                    "capitalize() takes no arguments (1 given)",
                                ));
                            }

                            let s = a[0].str();
                            // Lowercase the whole string with the Greek
                            // final-sigma rule, then title-map the first
                            // char (a ligature 'ﬁ' capitalizes to "Fi").
                            let lower = lower_with_final_sigma(&s);
                            let mut chars = lower.chars();
                            match chars.next() {
                                Some(first) => {
                                    let up: Vec<char> = first.to_uppercase().collect();
                                    let mut head = String::new();
                                    if let Some(h) = up.first() {
                                        head.push(*h);
                                        for r in up.iter().skip(1) {
                                            head.extend(r.to_lowercase());
                                        }
                                    }
                                    Ok(py_str(&(head + chars.as_str())))
                                }
                                None => Ok(py_str("")),
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "swapcase" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "swapcase".to_string(),
                        func: |a| {
                            if a.len() != 1 {
                                return Err(PyError::type_error(
                                    "swapcase() takes no arguments (1 given)",
                                ));
                            }

                            let s = a[0].str();
                            let chars: Vec<char> = s.chars().collect();
                            let mut result = String::with_capacity(s.len());
                            for (i, &c) in chars.iter().enumerate() {
                                if c.is_uppercase() || get_general_category(c) == GeneralCategory::TitlecaseLetter {
                                    for lc in c.to_lowercase() {
                                        if lc == '\u{03C3}' {
                                            result.push(if is_final_sigma(&chars, i) {
                                                '\u{03C2}'
                                            } else {
                                                '\u{03C3}'
                                            });
                                        } else {
                                            result.push(lc);
                                        }
                                    }
                                } else if c.is_lowercase() {
                                    result.extend(c.to_uppercase());
                                } else {
                                    result.push(c);
                                }
                            }
                            Ok(py_str(&result))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "zfill" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "zfill".to_string(),
                        func: |a| {
                            if a.len() < 2 {
                                return Err(PyError::type_error(
                                    "zfill() takes exactly 1 argument",
                                ));
                            }
                            let w = a[1].as_i64().unwrap_or(0) as usize;
                            let s = a[0].str();
                            if w <= s.len() {
                                return Ok(py_str(&s));
                            }
                            let (sign, rest) = if let Some(stripped) =
                                s.strip_prefix('+').or_else(|| s.strip_prefix('-'))
                            {
                                (&s[..1], stripped)
                            } else {
                                ("", s.as_str())
                            };
                            let padded =
                                format!("{}{:0>width$}", sign, rest, width = w - sign.len());
                            Ok(py_str(&padded))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "ljust" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "ljust".to_string(),
                        func: |a| {
                            if a.len() < 2 {
                                return Err(PyError::type_error(
                                    "ljust() takes exactly 1 argument",
                                ));
                            } else {
                                let w = a[1].as_i64().unwrap_or(0) as usize;
                                let fill = if a.len() > 2 {
                                    let f = a[2].str();
                                    f.chars().next().unwrap_or(' ')
                                } else {
                                    ' '
                                };
                                let s = a[0].str();
                                let padding = if w > s.len() {
                                    fill.to_string().repeat(w - s.len())
                                } else {
                                    String::new()
                                };
                                Ok(py_str(&(s.to_string() + &padding)))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "rjust" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "rjust".to_string(),
                        func: |a| {
                            if a.len() < 2 {
                                return Err(PyError::type_error(
                                    "rjust() takes exactly 1 argument",
                                ));
                            }
                            let w = a[1].as_i64().unwrap_or(0) as usize;
                            let fill = if a.len() > 2 {
                                a[2].str().chars().next().unwrap_or(' ')
                            } else {
                                ' '
                            };
                            let s = a[0].str();
                            if w <= s.len() {
                                Ok(py_str(&s))
                            } else {
                                Ok(py_str(&(fill.to_string().repeat(w - s.len()) + &s)))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "center" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "center".to_string(),
                        func: |a| {
                            if a.len() < 2 {
                                return Err(PyError::type_error(
                                    "center() takes exactly 1 argument",
                                ));
                            }
                            let w = a[1].as_i64().unwrap_or(0) as usize;
                            let fill = if a.len() > 2 {
                                a[2].str().chars().next().unwrap_or(' ')
                            } else {
                                ' '
                            };
                            let s = a[0].str();
                            if w <= s.len() {
                                Ok(py_str(&s))
                            } else {
                                let pad = w - s.len();
                                let left = pad / 2;
                                let right = pad - left;
                                let fill_s = fill.to_string();
                                Ok(py_str(&(fill_s.repeat(left) + &s + &fill_s.repeat(right))))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "removeprefix" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "removeprefix".to_string(),
                        func: |a| {
                            if a.len() != 2 {
                                return Err(PyError::type_error(
                                    "removeprefix() takes exactly one argument",
                                ));
                            }
                            if !matches!(&*a[1].borrow(), PyObject::Str(_)) {
                                return Err(PyError::type_error(format!(
                                    "removeprefix() argument must be str, not {}",
                                    a[1].borrow().type_name(),
                                )));
                            }
                            let s = a[0].str();
                            let p = a[1].str();
                            Ok(py_str(if s.starts_with(&p) { &s[p.len()..] } else { &s }))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "removesuffix" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "removesuffix".to_string(),
                        func: |a| {
                            if a.len() != 2 {
                                return Err(PyError::type_error(
                                    "removesuffix() takes exactly one argument",
                                ));
                            }
                            if !matches!(&*a[1].borrow(), PyObject::Str(_)) {
                                return Err(PyError::type_error(format!(
                                    "removesuffix() argument must be str, not {}",
                                    a[1].borrow().type_name(),
                                )));
                            }
                            let s = a[0].str();
                            let p = a[1].str();
                            Ok(py_str(if s.ends_with(&p) {
                                &s[..s.len() - p.len()]
                            } else {
                                &s
                            }))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__mod__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__mod__".to_string(),
                        func: |args| {
                            // args[0] = self_obj (py_none), args[1] = format string, args[2] = value
                            if args.len() < 3 {
                                return Err(PyError::type_error("__mod__() too few args"));
                            }
                            let fmt = args[1].str();
                            // Real CPython's `%`-formatting errors (bad
                            // conversion char, huge width/precision,
                            // mismatched mapping key, ...) are all
                            // `ValueError`, not `RuntimeError` — confirmed by
                            // `test_str.py`'s own `assertRaises(ValueError)`
                            // around several of these.
                            let result = string_interpolate(&fmt, &args[2]).map_err(|e| {
                                if e.contains("too big") || e.contains("[overflow]") {
                                    PyError::overflow_error(
                                        e.trim_end_matches(" [overflow]").to_string(),
                                    )
                                } else if e.contains("a real number is required")
                                    || e.contains("an integer is required")
                                    || e.contains("must be real number")
                                    || e.contains("not all arguments converted")
                                    || e.contains("requires an int or a unicode character")
                                {
                                    PyError::type_error(e)
                                } else {
                                    PyError::value_error(e)
                                }
                            })?;
                            Ok(py_str(&result))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "partition" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "partition".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "partition() takes exactly one argument",
                                ));
                            }
                            let s = args[0].str();
                            let sep = args[1].str();
                            if let Some(pos) = s.find(&sep) {
                                Ok(py_tuple(vec![
                                    py_str(&s[..pos]),
                                    py_str(&sep),
                                    py_str(&s[pos + sep.len()..]),
                                ]))
                            } else {
                                Ok(py_tuple(vec![py_str(&s), py_str(""), py_str("")]))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "rpartition" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "rpartition".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "rpartition() takes exactly one argument",
                                ));
                            }
                            let s = args[0].str();
                            let sep = args[1].str();
                            if let Some(pos) = s.rfind(&sep) {
                                Ok(py_tuple(vec![
                                    py_str(&s[..pos]),
                                    py_str(&sep),
                                    py_str(&s[pos + sep.len()..]),
                                ]))
                            } else {
                                Ok(py_tuple(vec![py_str(""), py_str(""), py_str(&s)]))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "splitlines" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "splitlines".to_string(),
                        func: |args| {
                            let s = args[0].str();
                            let keepends = if args.len() > 1 {
                                args[1].truthy()
                            } else {
                                false
                            };
                            let mut lines: Vec<PyObjectRef> = Vec::new();
                            let mut start = 0;
                            let chars: Vec<char> = s.chars().collect();
                            let len = chars.len();
                            let mut i = 0;
                            while i < len {
                                let end;
                                let line_end;
                                if chars[i] == '\r' {
                                    if i + 1 < len && chars[i + 1] == '\n' {
                                        line_end = i + 2;
                                    } else {
                                        line_end = i + 1;
                                    }
                                } else if chars[i] == '\n' {
                                    line_end = i + 1;
                                } else {
                                    i += 1;
                                    continue;
                                }
                                if keepends {
                                    end = line_end;
                                } else {
                                    end = i;
                                }
                                let line: String = chars[start..end].iter().collect();
                                lines.push(py_str(&line));
                                i = line_end;
                                start = i;
                            }
                            // A trailing chunk is only pushed if there's
                            // actual leftover content after the last
                            // line-terminator (`start < len`) — matching
                            // `bytes.splitlines()`'s own, already-correct
                            // logic just above in this file. This used to
                            // ALSO push a chunk whenever the string ended
                            // with `\n` or was empty, backwards from real
                            // Python semantics: `"a\nb\n".splitlines()`
                            // must be `['a', 'b']` (NOT `['a', 'b', '']`
                            // — a trailing newline does not create an
                            // extra empty final line) and `"".splitlines()`
                            // must be `[]` (NOT `['']`). Confirmed via
                            // `test_augassign.py::testCustomMethods2`,
                            // which compares a captured call-log list
                            // against a multi-line string literal's
                            // `.splitlines()` — the literal's trailing
                            // newline (before the closing `'''`) produced
                            // one spurious extra `''` element, permanently
                            // failing the comparison regardless of the
                            // actual dunder-call behavior being tested.
                            if start < len {
                                let line: String = chars[start..].iter().collect();
                                lines.push(py_str(&line));
                            }
                            Ok(py_list(lines))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "expandtabs" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "expandtabs".to_string(),
                        func: |args| {
                            let s = args[0].str();
                            let tabsize = if args.len() > 1 {
                                args[1].as_i64().unwrap_or(8) as usize
                            } else {
                                8
                            };
                            let mut result = String::with_capacity(s.len());
                            let mut col = 0;
                            for c in s.chars() {
                                if c == '\t' {
                                    let spaces = tabsize - (col % tabsize);
                                    result.push_str(&" ".repeat(spaces));
                                    col += spaces;
                                } else if c == '\n' || c == '\r' {
                                    result.push(c);
                                    col = 0;
                                } else {
                                    result.push(c);
                                    col += 1;
                                }
                            }
                            Ok(py_str(&result))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "translate" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "translate".to_string(),
                        func: |a| {
                            let s = a[0].str();
                            if a.len() < 2 || matches!(&*a[1].borrow(), PyObject::None) {
                                return Ok(py_str(&s));
                            }
                            let table = a[1].clone();
                            let mut result = String::new();
                            for ch in s.chars() {
                                let key_int = py_int(ch as i64);
                                let replacement = match &*table.borrow() {
                                    PyObject::Dict(d) => d.get(&key_int).ok().flatten(),
                                    _ => None,
                                };
                                match replacement {
                                    None => result.push(ch),
                                    Some(r) => {
                                        if matches!(&*r.borrow(), PyObject::None) {
                                        } else if let Some(ord) = r.as_i64() {
                                            if let Some(c) = char::from_u32(ord as u32) {
                                                result.push(c);
                                            } else {
                                                result.push_str(&r.str());
                                            }
                                        } else if let PyObject::Str(ss) = &*r.borrow() {
                                            result.push_str(ss);
                                        } else {
                                            result.push_str(&r.str());
                                        }
                                    }
                                }
                            }
                            Ok(py_str(&result))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "maketrans" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "maketrans".to_string(),
                        func: |a| crate::object::str_maketrans_builtin(&a[1..]),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "encode" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "encode".to_string(),
                        func: str_encode_builtin,
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "isidentifier" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "isidentifier".to_string(),
                        func: |a| {
                            if a.len() != 1 {
                                return Err(PyError::type_error(
                                    "isidentifier() takes no arguments (1 given)",
                                ));
                            }

                            let s = a[0].str();
                            if s.is_empty() {
                                return Ok(py_bool(false));
                            }
                            let mut chars = s.chars();
                            let first = chars.next().unwrap();
                            let valid = (first == '_') || first.is_alphabetic();
                            if !valid {
                                return Ok(py_bool(false));
                            }
                            Ok(py_bool(
                                chars.all(|c| c == '_' || c.is_alphanumeric()),
                            ))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__sizeof__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__sizeof__".to_string(),
                        func: |a| {
                            let s = a[0].str();
                            Ok(py_int(49 + s.len() as i64))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    // Same gap, same fix, as `list`'s own `__getitem__` arm.
                    "__getitem__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__getitem__".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "__getitem__() takes exactly one argument",
                                ));
                            }
                            py_getitem(&args[0], &args[1])
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__len__" => {
                        let len = _s.chars().count() as i64;
                        Ok(PyObjectRef::new(PyObject::Closure(Rc::new(
                            move |_args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                                Ok(py_int(len))
                            },
                        ))))
                    }
                    "__iter__" => {
                        let str_clone = _s.clone();
                        Ok(PyObjectRef::new(PyObject::Closure(Rc::new(
                            move |_args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                                crate::object::builtin_iter(&[PyObjectRef::new(PyObject::Str(str_clone.clone()))])
                            },
                        ))))
                    }
                    "format_map" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "format_map".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error("format_map() takes exactly one argument"));
                            }
                            let fmt = args[0].str();
                            let mapping = args[1].clone();
                            let mut result = String::new();
                            let mut chars = fmt.chars();
                            while let Some(c) = chars.next() {
                                if c == '{' {
                                    if chars.as_str().starts_with('{') { result.push('{'); chars.next(); continue; }
                                    let mut field = String::new();
                                    let mut depth = 0usize;
                                    loop {
                                        match chars.next() {
                                            Some('}') if depth==0 => break,
                                            Some('}') => { depth-=1; field.push('}'); }
                                            Some('{') => { depth+=1; field.push('{'); }
                                            Some(c) => field.push(c),
                                            None => return Err(PyError::value_error("unterminated format field")),
                                        }
                                    }
                                    let (name_part, spec) = match field.find(':') {
                                        Some(idx) => (&field[..idx], &field[idx+1..]),
                                        None => (field.as_str(), ""),
                                    };
                                    let (name_part, conv) = match name_part.find('!') {
                                        Some(idx) => (&name_part[..idx], Some(&name_part[idx+1..])),
                                        None => (name_part, None),
                                    };
                                    if name_part.is_empty() {
                                        return Err(PyError::value_error("empty field name in format_map"));
                                    }
                                    if let Ok(_) = name_part.parse::<usize>() {
                                        return Err(PyError::value_error("format_map() positional field not allowed"));
                                    }
                                    let val: PyObjectRef = if let Some(br) = name_part.find('[') {
                                        let base = &name_part[..br];
                                        let key = &name_part[br+1..name_part.len()-1];
                                        let base_val = if let PyObject::Dict(d) = &*mapping.borrow() {
                                            d.get(&py_str(base)).ok().flatten().ok_or_else(|| PyError::key_error(format!("'{}'", base)))?
                                        } else { return Err(PyError::type_error("format_map() argument must be dict")); };
                                        py_getitem(&base_val, &py_str(key)).or_else(|_| {
                                            if let Ok(idx) = key.parse::<usize>() {
                                                py_getitem(&base_val, &py_int(idx as i64))
                                            } else { Err(PyError::key_error(format!("'{}'", key))) }
                                        })?
                                    } else {
                                        if let PyObject::Dict(d) = &*mapping.borrow() {
                                            d.get(&py_str(name_part)).ok().flatten().ok_or_else(|| PyError::key_error(format!("'{}'", name_part)))?
                                        } else { return Err(PyError::type_error("format_map() argument must be dict")); }
                                    };
                                    let val = match conv {
                                        Some("r") | Some("a") => py_str(&val.borrow().repr()),
                                        Some("s") => py_str(&val.str()),
                                        _ => val,
                                    };
                                    result.push_str(&crate::vm::format_with_spec(&val, spec)?);
                                } else if c == '}' {
                                    if chars.as_str().starts_with('}') { result.push('}'); chars.next(); }
                                } else { result.push(c); }
                            }
                            Ok(py_str(&result))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__getnewargs__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__getnewargs__".to_string(),
                        func: |args| {
                            let s = args[0].str();
                            Ok(py_tuple(vec![py_str(&s)]))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__getnewargs_ex__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__getnewargs_ex__".to_string(),
                        func: |args| {
                            let s = args[0].str();
                            Ok(py_tuple(vec![py_tuple(vec![py_str(&s)]), py_dict()]))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    _ => Err(PyError::attribute_error(format!(
                        "'str' object has no attribute '{}'",
                        name
                    ))),
                }
            }
        _ => unreachable!("{} called with non-{} object", stringify!(get), o.type_name()),
    }
}
