// Auto-extracted from src/object/attrs/mod.rs lines 5014-5779
use crate::object::*;
use super::*;
use crate::interner;

pub(crate) fn get(o: &PyObject, name: &str) -> PyResult<PyObjectRef> {
    match o {
            PyObject::Str(_s) => {
                match name {
                    "replace" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "replace".to_string(),
                        func: |args| {
                            if args.len() < 3 {
                                return Err(PyError::type_error(
                                    "replace() takes exactly 2 arguments",
                                ));
                            }
                            Ok(py_str(
                                &args[0].str().replace(&args[1].str(), &args[2].str()),
                            ))
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

                            Ok(py_bool(
                                !a[0].str().is_empty()
                                    && a[0]
                                        .str()
                                        .chars()
                                        .all(|c| c.is_ascii_digit() && !c.is_ascii_control()),
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

                            Ok(py_bool(
                                !a[0].str().is_empty()
                                    && a[0].str().chars().any(|c| c.is_numeric()),
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

                            Ok(py_bool(
                                !a[0].str().is_empty()
                                    && a[0].str().chars().all(|c| c.is_ascii_graphic() || c == ' '),
                            ))
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
                            Ok(py_str(&a[0].str().to_lowercase()))
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
                            Ok(py_bool(a[0].str().chars().all(|c| c.is_ascii_digit())))
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
                            Ok(py_bool(a[0].str().chars().all(|c| c.is_ascii_alphabetic())))
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

                            if a.len() != 1 {
                                return Err(PyError::type_error(
                                    "isalpha() takes no arguments (1 given)",
                                ));
                            }

                            Ok(py_bool(
                                a[0].str().chars().all(|c| c.is_ascii_alphanumeric()),
                            ))
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
                            Ok(py_bool(a[0].str().chars().all(|c| c.is_whitespace())))
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
                            Ok(py_bool(a[0].str() == a[0].str().to_lowercase()))
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
                            Ok(py_bool(a[0].str() == a[0].str().to_uppercase()))
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
                            let mut prev_is_letter = false;
                            let mut is_title = true;
                            for c in s.chars() {
                                if c.is_ascii_uppercase() {
                                    if prev_is_letter {
                                        is_title = false;
                                        break;
                                    }
                                    prev_is_letter = true;
                                } else if c.is_ascii_lowercase() {
                                    if !prev_is_letter {
                                        is_title = false;
                                        break;
                                    }
                                    prev_is_letter = true;
                                } else {
                                    prev_is_letter = false;
                                }
                            }
                            Ok(py_bool(is_title && !s.is_empty()))
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
                            let mut result = String::with_capacity(s.len());
                            let mut prev_cased = false;
                            for c in s.chars() {
                                if c.is_uppercase() || c.is_lowercase() {
                                    if !prev_cased {
                                        // CPython's str.title uses the TITLE
                                        // case mapping (a ligature '\uFB01'
                                        // becomes "Fi", not "FI"): take the
                                        // uppercase expansion and lowercase
                                        // every char after the first.
                                        let up: Vec<char> = c.to_uppercase().collect();
                                        if let Some(first) = up.first() {
                                            result.push(*first);
                                            for rest in up.iter().skip(1) {
                                                result.extend(rest.to_lowercase());
                                            }
                                        }
                                    } else {
                                        result.extend(c.to_lowercase());
                                    }
                                    prev_cased = true;
                                } else {
                                    result.push(c);
                                    prev_cased = false;
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
                            let cased = |c: &char| c.is_uppercase() || c.is_lowercase();
                            for (i, &c) in chars.iter().enumerate() {
                                if c.is_uppercase() {
                                    // A capital sigma lowercases to final
                                    // sigma (U+03C2) at word end, else U+03C3.
                                    for lc in c.to_lowercase() {
                                        if lc == '\u{03C3}' {
                                            let prev_cased = i > 0 && cased(&chars[i - 1]);
                                            let next_cased =
                                                i + 1 < chars.len() && cased(&chars[i + 1]);
                                            result.push(if prev_cased && !next_cased {
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
                            // `str.translate(table)` — `table` maps Unicode ordinal (int) or
                            // single-char string to ordinal, string, or None (delete). Must
                            // handle both int and str keys, and int/str/None values.
                            let table = a[1].clone();
                            let mut result = String::new();
                            for ch in s.chars() {
                                let key_str = py_str(&ch.to_string());
                                let key_int = py_int(ch as i64);
                                let replacement = match &*table.borrow() {
                                    PyObject::Dict(d) => {
                                        if let Some(v) = d.get(&key_int).ok().flatten() {
                                            Some(v)
                                        } else {
                                            d.get(&key_str).ok().flatten()
                                        }
                                    }
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
                            let valid = (first == '_') || first.is_ascii_alphabetic();
                            if !valid {
                                return Ok(py_bool(false));
                            }
                            Ok(py_bool(
                                chars.all(|c| c == '_' || c.is_ascii_alphanumeric()),
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
                    _ => Err(PyError::attribute_error(format!(
                        "'str' object has no attribute '{}'",
                        name
                    ))),
                }
            }
        _ => unreachable!("{} called with non-{} object", stringify!(get), o.type_name()),
    }
}
