use crate::object::*;
use std::collections::HashMap;

pub fn create_textwrap_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! tw_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    tw_func!("dedent", |args| {
        if args.len() < 1 {
            return Err(PyError::type_error("dedent() takes exactly 1 argument"));
        }
        let text = args[0].str();
        let indent = text.lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| l.len() - l.trim_start().len())
            .min()
            .unwrap_or(0);
        let result: String = text.lines()
            .map(|l| {
                if l.len() >= indent && l.chars().take(indent).all(|c| c.is_whitespace()) {
                    &l[indent..]
                } else {
                    l
                }
            })
            .collect::<Vec<&str>>()
            .join("\n");
        Ok(py_str(&result))
    });

    tw_func!("indent", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("indent() takes at least 2 arguments"));
        }
        let text = args[0].str();
        let prefix = args[1].str();
        let result: String = text.lines()
            .map(|l| format!("{}{}", prefix, l))
            .collect::<Vec<String>>()
            .join("\n");
        Ok(py_str(&result))
    });

    tw_func!("fill", |args| {
        if args.len() < 1 {
            return Err(PyError::type_error("fill() takes at least 1 argument"));
        }
        let text = args[0].str();
        let width = if args.len() > 2 {
            args[2].as_i64().unwrap_or(70) as usize
        } else {
            70
        };
        if width == 0 || width >= text.len() {
            return Ok(py_str(&text));
        }
        let words: Vec<&str> = text.split_whitespace().collect();
        let mut lines: Vec<String> = Vec::new();
        let mut current = String::new();
        for word in words {
            if current.is_empty() {
                current = word.to_string();
            } else if current.len() + 1 + word.len() <= width {
                current.push(' ');
                current.push_str(word);
            } else {
                lines.push(current);
                current = word.to_string();
            }
        }
        if !current.is_empty() {
            lines.push(current);
        }
        Ok(py_str(&lines.join("\n")))
    });

    tw_func!("shorten", |args| {
        if args.len() < 1 {
            return Err(PyError::type_error("shorten() takes at least 1 argument"));
        }
        let text = args[0].str();
        let width = if args.len() > 1 {
            args[1].as_i64().unwrap_or(70) as usize
        } else {
            70
        };
        if text.len() <= width {
            return Ok(py_str(&text));
        }
        let truncated: String = text.chars().take(width).collect();
        if let Some(last_space) = truncated.rfind(' ') {
            let result: String = truncated[..last_space].to_string() + " ...";
            Ok(py_str(&result))
        } else {
            Ok(py_str(&(truncated + " ...")))
        }
    });

    d
}

pub fn create_pprint_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! pp_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    fn pprint_recurse(obj: &PyObjectRef, indent: usize, out: &mut String) {
        let borrowed = obj.borrow();
        match &*borrowed {
            PyObject::List(items) => {
                if items.is_empty() {
                    out.push_str("[]");
                    return;
                }
                out.push_str("[\n");
                for (i, item) in items.iter().enumerate() {
                    out.push_str(&" ".repeat(indent + 4));
                    pprint_recurse(item, indent + 4, out);
                    if i < items.len() - 1 { out.push(','); }
                    out.push('\n');
                }
                out.push_str(&" ".repeat(indent));
                out.push(']');
            }
            PyObject::Tuple(items) => {
                if items.is_empty() {
                    out.push_str("()");
                    return;
                }
                if items.len() == 1 {
                    out.push_str("(\n");
                    out.push_str(&" ".repeat(indent + 4));
                    pprint_recurse(&items[0], indent + 4, out);
                    out.push_str(",\n");
                    out.push_str(&" ".repeat(indent));
                    out.push(')');
                    return;
                }
                out.push_str("(\n");
                for (i, item) in items.iter().enumerate() {
                    out.push_str(&" ".repeat(indent + 4));
                    pprint_recurse(item, indent + 4, out);
                    if i < items.len() - 1 { out.push(','); }
                    out.push('\n');
                }
                out.push_str(&" ".repeat(indent));
                out.push(')');
            }
            PyObject::Dict(dict) => {
                if dict.is_empty() {
                    out.push_str("{}");
                    return;
                }
                out.push_str("{\n");
                let pairs = dict.items();
                for (i, (k, v)) in pairs.iter().enumerate() {
                    out.push_str(&" ".repeat(indent + 4));
                    pprint_recurse(k, indent + 4, out);
                    out.push_str(": ");
                    pprint_recurse(v, indent + 4, out);
                    if i < pairs.len() - 1 { out.push(','); }
                    out.push('\n');
                }
                out.push_str(&" ".repeat(indent));
                out.push('}');
            }
            PyObject::Set(items) => {
                let vec = items.to_vec();
                if vec.is_empty() {
                    out.push_str("set()");
                    return;
                }
                out.push_str("{\n");
                for (i, item) in vec.iter().enumerate() {
                    out.push_str(&" ".repeat(indent + 4));
                    pprint_recurse(item, indent + 4, out);
                    if i < vec.len() - 1 { out.push(','); }
                    out.push('\n');
                }
                out.push_str(&" ".repeat(indent));
                out.push('}');
            }
            PyObject::Str(s) => {
                out.push('\'');
                out.push_str(s);
                out.push('\'');
            }
            _ => {
                out.push_str(&borrowed.repr());
            }
        }
    }

    pp_func!("pprint", |args| {
        if args.len() < 1 {
            return Err(PyError::type_error("pprint() takes at least 1 argument"));
        }
        let mut out = String::new();
        pprint_recurse(&args[0], 0, &mut out);
        print!("{}", out);
        Ok(py_none())
    });

    pp_func!("pformat", |args| {
        if args.len() < 1 {
            return Err(PyError::type_error("pformat() takes at least 1 argument"));
        }
        let mut out = String::new();
        pprint_recurse(&args[0], 0, &mut out);
        Ok(py_str(&out))
    });

    pp_func!("isreadable", |args| {
        if args.len() < 1 {
            return Err(PyError::type_error("isreadable() takes at least 1 argument"));
        }
        let obj = &args[0];
        let borrowed = obj.borrow();
        fn is_simple_literal(obj: &PyObject) -> bool {
            matches!(obj,
                PyObject::Int(_) | PyObject::Float(_) | PyObject::Bool(_)
                | PyObject::Str(_) | PyObject::Bytes(_) | PyObject::None
            )
        }
        let readable = match &*borrowed {
            PyObject::List(items) => items.iter().all(|item| is_simple_literal(&item.borrow())),
            PyObject::Tuple(items) => items.iter().all(|item| is_simple_literal(&item.borrow())),
            PyObject::Set(items) => items.to_vec().iter().all(|item| is_simple_literal(&item.borrow())),
            PyObject::FrozenSet(items) => items.to_vec().iter().all(|item| is_simple_literal(&item.borrow())),
            PyObject::Dict(dict) => {
                dict.items().iter().all(|(k, v)| {
                    is_simple_literal(&k.borrow()) && is_simple_literal(&v.borrow())
                })
            }
            _ => is_simple_literal(&borrowed),
        };
        Ok(PyObjectRef::SmallBool(readable))
    });

    d
}

pub fn create_string_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    let ascii_lowercase = "abcdefghijklmnopqrstuvwxyz";
    let ascii_uppercase = "ABCDEFGHIJKLMNOPQRSTUVWXYZ";
    let ascii_letters = &format!("{}{}", ascii_lowercase, ascii_uppercase);
    let digits = "0123456789";
    let hexdigits = "0123456789abcdefABCDEF";
    let octdigits = "01234567";
    let punctuation = "!\"#$%&'()*+,-./:;<=>?@[\\]^_`{|}~";
    let whitespace = " \t\n\r\u{0b}\u{0c}";
    let printable = &format!("{}{}{}{}", digits, ascii_letters, punctuation, whitespace);

    d.insert("ascii_letters".to_string(), py_str(ascii_letters));
    d.insert("ascii_lowercase".to_string(), py_str(ascii_lowercase));
    d.insert("ascii_uppercase".to_string(), py_str(ascii_uppercase));
    d.insert("digits".to_string(), py_str(digits));
    d.insert("hexdigits".to_string(), py_str(hexdigits));
    d.insert("octdigits".to_string(), py_str(octdigits));
    d.insert("punctuation".to_string(), py_str(punctuation));
    d.insert("printable".to_string(), py_str(printable));
    d.insert("whitespace".to_string(), py_str(whitespace));

    d
}

pub fn create_reprlib_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    d.insert("repr".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "repr".to_string(),
        func: |args| {
            if args.is_empty() {
                return Err(PyError::type_error("repr() missing required argument"));
            }
            let s = if args.len() > 1 {
                let limit = args[1].as_i64().unwrap_or(80) as usize;
                let obj_repr = args[0].repr();
                if obj_repr.len() > limit {
                    if limit > 3 {
                        format!("{}...", &obj_repr[..limit-3])
                    } else {
                        obj_repr[..limit].to_string()
                    }
                } else {
                    obj_repr
                }
            } else {
                let obj_repr = args[0].repr();
                if obj_repr.len() > 80 {
                    format!("{}...", &obj_repr[..77])
                } else {
                    obj_repr
                }
            };
            Ok(py_str(&s))
        },
    }));
    d
}

pub fn create_mimetypes_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    d.insert("guess_type".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "guess_type".to_string(), func: mime_guess_type }));
    d.insert("guess_extension".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "guess_extension".to_string(), func: mime_guess_extension }));
    d.insert("add_type".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "add_type".to_string(), func: mime_add_type }));
    // list of known types, init, read_mime_types, etc. can be added as needed
    d.insert("known_types".to_string(), py_dict());
    d.insert("inited".to_string(), py_bool(false));
    d
}

pub fn create_string_dict_v2() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! str_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    // capwords(s, sep=None) — split into words, capitalize each, join
    str_func!("capwords", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("capwords() missing required argument: s"));
        }
        let s = args[0].str();

        let result = if args.len() > 1 {
            let sep_str = args[1].str();
            if sep_str.is_empty() {
                // Default whitespace splitting
                let words: Vec<String> = s.split_whitespace()
                    .map(|w| {
                        let mut chars = w.chars();
                        match chars.next() {
                            None => String::new(),
                            Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
                        }
                    })
                    .collect();
                words.join(" ")
            } else {
                let words: Vec<String> = s.split(&sep_str)
                    .map(|w| {
                        let trimmed = w.trim();
                        if trimmed.is_empty() {
                            String::new()
                        } else {
                            let mut chars = trimmed.chars();
                            match chars.next() {
                                None => String::new(),
                                Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
                            }
                        }
                    })
                    .collect();
                words.join(&sep_str)
            }
        } else {
            // Default: split by whitespace, capitalize, join with space
            let words: Vec<String> = s.split_whitespace()
                .map(|w| {
                    let mut chars = w.chars();
                    match chars.next() {
                        None => String::new(),
                        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
                    }
                })
                .collect();
            words.join(" ")
        };

        Ok(py_str(&result))
    });

    // Formatter class stub
    let formatter = PyObjectRef::new(PyObject::BuiltinFunction {
        name: "Formatter".to_string(),
        func: |_args| {
            let mut dict = HashMap::new();

            dict.insert("vformat".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
                name: "vformat".to_string(),
                func: |_| Ok(py_str("vformat stub")),
            }));

            dict.insert("format".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
                name: "format".to_string(),
                func: |fargs| {
                    if fargs.is_empty() { return Ok(py_str("")); }
                    Ok(py_str(&fargs[0].str()))
                },
            }));

            dict.insert("parse".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
                name: "parse".to_string(),
                func: |_| Ok(py_list(vec![])),
            }));

            dict.insert("get_field".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
                name: "get_field".to_string(),
                func: |_| Ok(py_str("")),
            }));

            dict.insert("get_value".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
                name: "get_value".to_string(),
                func: |_| Ok(py_str("")),
            }));

            dict.insert("check_unused_args".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
                name: "check_unused_args".to_string(),
                func: |_| Ok(py_none()),
            }));

            dict.insert("format_field".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
                name: "format_field".to_string(),
                func: |fargs| {
                    if fargs.is_empty() { return Ok(py_str("")); }
                    Ok(py_str(&fargs[0].str()))
                },
            }));

            dict.insert("convert_field".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
                name: "convert_field".to_string(),
                func: |fargs| {
                    if fargs.is_empty() { return Ok(py_str("")); }
                    Ok(fargs[0].clone())
                },
            }));

            Ok(PyObjectRef::new(PyObject::Instance {
                typ: py_str("Formatter"),
                dict,
            }))
        },
    });

    d.insert("Formatter".to_string(), formatter);
    d
}

pub fn create_difflib_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! dfl_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    // Helper: compute LCS length table for two sequences
    fn lcs_table(a: &[&str], b: &[&str]) -> Vec<Vec<usize>> {
        let m = a.len();
        let n = b.len();
        let mut dp = vec![vec![0usize; n + 1]; m + 1];
        for i in 1..=m {
            for j in 1..=n {
                if a[i-1] == b[j-1] {
                    dp[i][j] = dp[i-1][j-1] + 1;
                } else {
                    dp[i][j] = dp[i-1][j].max(dp[i][j-1]);
                }
            }
        }
        dp
    }

    // Backtrack to get the edit operations
    fn backtrack<'a>(a: &'a [&str], b: &'a [&str], dp: &[Vec<usize>]) -> Vec<(char, &'a str)> {
        let mut ops = Vec::new();
        let mut i = a.len();
        let mut j = b.len();
        while i > 0 || j > 0 {
            if i > 0 && j > 0 && a[i-1] == b[j-1] {
                ops.push((' ', a[i-1]));
                i -= 1;
                j -= 1;
            } else if j > 0 && (i == 0 || dp[i][j-1] >= dp[i-1][j]) {
                ops.push(('+', b[j-1]));
                j -= 1;
            } else if i > 0 {
                ops.push(('-', a[i-1]));
                i -= 1;
            }
        }
        ops.reverse();
        ops
    }

    dfl_func!("unified_diff", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("unified_diff() requires at least 2 arguments (a, b)"));
        }

        fn extract_lines(obj: &PyObjectRef) -> PyResult<Vec<String>> {
            let borrowed = obj.borrow();
            match &*borrowed {
                PyObject::Str(s) => Ok(s.lines().map(|l| l.to_string()).collect()),
                PyObject::List(items) => {
                    items.iter().map(|item| Ok(item.str())).collect()
                }
                _ => Err(PyError::type_error("arguments to unified_diff() must be strings or lists of strings")),
            }
        }

        let a_lines = extract_lines(&args[0])?;
        let b_lines = extract_lines(&args[1])?;

        let a_refs: Vec<&str> = a_lines.iter().map(|s| s.as_str()).collect();
        let b_refs: Vec<&str> = b_lines.iter().map(|s| s.as_str()).collect();

        let mut result: Vec<PyObjectRef> = Vec::new();

        if a_refs == b_refs {
            return Ok(py_list(vec![]));
        }

        result.push(py_str("--- a"));
        result.push(py_str("+++ b"));

        let dp = lcs_table(&a_refs, &b_refs);
        let ops = backtrack(&a_refs, &b_refs, &dp);

        // Build hunks from ops
        let mut hunks: Vec<(usize, usize, Vec<(char, String)>)> = Vec::new();
        let mut current_hunk: Vec<(char, String)> = Vec::new();
        let mut a_pos = 0usize;
        let mut b_pos = 0usize;
        let mut hunk_a_start = 0usize;
        let mut hunk_b_start = 0usize;
        let mut in_hunk = false;

        for (op, line) in ops {
            match op {
                ' ' => {
                    if !current_hunk.is_empty() {
                        // Check if we have enough changes to flush
                        if current_hunk.len() >= 2 {
                            hunks.push((hunk_a_start, hunk_b_start, current_hunk.clone()));
                        }
                        current_hunk.clear();
                        in_hunk = false;
                    }
                    a_pos += 1;
                    b_pos += 1;
                }
                _ => {
                    if !in_hunk {
                        hunk_a_start = a_pos;
                        hunk_b_start = b_pos;
                        in_hunk = true;
                    }
                    current_hunk.push((op, line.to_string()));
                    if op == '-' {
                        a_pos += 1;
                    } else {
                        b_pos += 1;
                    }
                }
            }
        }

        // Flush last hunk
        if !current_hunk.is_empty() {
            hunks.push((hunk_a_start, hunk_b_start, current_hunk));
        }

        for (hunk_a_start, hunk_b_start, hunk_lines) in &hunks {
            let ctx_before = if *hunk_a_start > 3 { 3 } else { *hunk_a_start };
            let ctx_after = 0usize;

            let hunk_a_len = hunk_lines.iter().filter(|(op, _)| *op != '+').count() + ctx_before + ctx_after;
            let hunk_b_len = hunk_lines.iter().filter(|(op, _)| *op != '-').count() + ctx_before + ctx_after;

            result.push(py_str(&format!("@@ -{},{} +{},{} @@",
                hunk_a_start + 1 - ctx_before,
                if hunk_a_len == 0 { 0 } else { hunk_a_len },
                hunk_b_start + 1 - ctx_before,
                if hunk_b_len == 0 { 0 } else { hunk_b_len },
            )));

            // Add context before
            for k in (hunk_a_start.saturating_sub(ctx_before))..*hunk_a_start {
                if k < a_refs.len() {
                    result.push(py_str(&format!(" {}", a_refs[k])));
                }
            }

            for (op, line) in hunk_lines {
                result.push(py_str(&format!("{}{}", op, line)));
            }
        }

        Ok(py_list(result))
    });

    // Also add SequenceMatcher class (stub)
    let seq_matcher = PyObjectRef::new(PyObject::BuiltinFunction {
        name: "SequenceMatcher".to_string(),
        func: |_args| {
            let mut dict = HashMap::new();
            dict.insert("ratio".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
                name: "ratio".to_string(),
                func: |_| Ok(py_float(1.0)),
            }));
            Ok(PyObjectRef::new(PyObject::Instance {
                typ: py_str("SequenceMatcher"),
                dict,
            }))
        },
    });
    d.insert("SequenceMatcher".to_string(), seq_matcher);

    dfl_func!("get_close_matches", |args| {
        let word = if args.len() > 0 { args[0].str() } else { return Err(PyError::type_error("get_close_matches() requires at least 1 argument")); };
        // Return empty list (simple stub — doesn't implement actual matching)
        Ok(py_list(vec![]))
    });

    d
}

pub fn create_html_parser_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();

    thread_local! {
        static HTML_PARSER_DATA: std::cell::RefCell<String> = std::cell::RefCell::new(String::new());
    }

    // HTMLParser class — callable that returns an instance with feed, close, getpos
    let html_parser = PyObjectRef::new(PyObject::BuiltinFunction {
        name: "HTMLParser".to_string(),
        func: |_args| {
            let mut dict = HashMap::new();

            // feed(data) — accumulates data
            dict.insert("feed".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
                name: "feed".to_string(),
                func: |fargs| {
                    if !fargs.is_empty() {
                        HTML_PARSER_DATA.with(|d| {
                            d.borrow_mut().push_str(&fargs[0].str());
                        });
                    }
                    Ok(py_none())
                },
            }));

            // close() — returns accumulated data and clears
            dict.insert("close".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
                name: "close".to_string(),
                func: |_| {
                    let result = HTML_PARSER_DATA.with(|d| d.borrow().clone());
                    HTML_PARSER_DATA.with(|d| d.borrow_mut().clear());
                    Ok(py_str(&result))
                },
            }));

            // getpos() — returns (1, 0)
            dict.insert("getpos".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
                name: "getpos".to_string(),
                func: |_| Ok(py_tuple(vec![py_int(1), py_int(0)])),
            }));

            Ok(PyObjectRef::new(PyObject::Instance {
                typ: py_str("HTMLParser"),
                dict,
            }))
        },
    });
    d.insert("HTMLParser".to_string(), html_parser);

    d
}

use std::rc::Rc;
use std::cell::RefCell;
use num_traits::ToPrimitive;