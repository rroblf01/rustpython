use crate::object::*;
use std::collections::HashMap;

pub fn create_ast_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    // compile() flags (CPython's ast.PyCF_* constants) — test_builtin's
    // test_compile_top_level_await references PyCF_ALLOW_TOP_LEVEL_AWAIT.
    d.insert("PyCF_ONLY_AST".to_string(), py_int(0x40));
    d.insert("PyCF_ALLOW_TOP_LEVEL_AWAIT".to_string(), py_int(0x8000));
    d.insert("PyCF_TYPE_COMMENTS".to_string(), py_int(0x1000));
    d.insert("PyCF_DONT_IMPLY_DEDENT".to_string(), py_int(0x200));
    d.insert("PyCF_ACCEPT_NULL_BYTES".to_string(), py_int(0x10000000));
    macro_rules! ast_func {
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

    // literal_eval — simplified parser handling common Python literals
    ast_func!("literal_eval", |args| {
        if args.is_empty() {
            return Err(PyError::type_error(
                "literal_eval() missing required argument: node_or_string",
            ));
        }
        let arg = &args[0];
        let s = arg.str();
        // Trim whitespace
        let s = s.trim().to_string();
        if s.is_empty() {
            return Err(PyError::ValueError(
                "malformed node or string: empty literal".to_string(),
            ));
        }

        // Try parsing as a literal from left to right
        let chars: Vec<char> = s.chars().collect();
        let mut pos = 0;
        let result = parse_literal(&chars, &mut pos)?;
        // Expect EOF after successful parse
        skip_ws(&chars, &mut pos);
        if pos < chars.len() {
            return Err(PyError::ValueError(format!(
                "malformed node or string: trailing garbage at position {}",
                pos
            )));
        }
        Ok(result)
    });

    d.insert_str("AST", py_str("AST"));
    d.insert_str("Node", py_str("Node"));
    d.insert_str("Expr", py_str("Expr"));
    d.insert_str("Module", py_str("Module"));
    d.insert_str("Load", py_str("Load"));
    d.insert_str("Store", py_str("Store"));
    d.insert_str("Del", py_str("Del"));
    d.insert_str("Pass", py_str("Pass"));
    d.insert_str("Break", py_str("Break"));
    d.insert_str("Continue", py_str("Continue"));

    d
}

/// Skip whitespace characters in the character slice.
fn skip_ws(chars: &[char], pos: &mut usize) {
    while *pos < chars.len() && chars[*pos].is_ascii_whitespace() {
        *pos += 1;
    }
}

/// Parse a single Python literal starting at `pos`.  Supports: strings,
/// integers, floats, True, False, None, tuples (...), lists [...], dicts {...}.
fn parse_literal(chars: &[char], pos: &mut usize) -> PyResult<PyObjectRef> {
    skip_ws(chars, pos);
    if *pos >= chars.len() {
        return Err(PyError::ValueError(
            "malformed node or string: unexpected end".to_string(),
        ));
    }

    match chars[*pos] {
        // String literal: simple quoted string (no escape sequences)
        '\'' | '"' => {
            let quote = chars[*pos];
            *pos += 1;
            let mut buf = String::new();
            loop {
                if *pos >= chars.len() {
                    return Err(PyError::ValueError(
                        "malformed node or string: unterminated string".to_string(),
                    ));
                }
                let c = chars[*pos];
                *pos += 1;
                if c == quote {
                    break;
                }
                if c == '\\' && *pos < chars.len() {
                    // Handle common escape sequences
                    let next = chars[*pos];
                    *pos += 1;
                    match next {
                        'n' => buf.push('\n'),
                        't' => buf.push('\t'),
                        'r' => buf.push('\r'),
                        '\\' => buf.push('\\'),
                        '\'' => buf.push('\''),
                        '"' => buf.push('"'),
                        _ => {
                            buf.push('\\');
                            buf.push(next);
                        }
                    }
                } else {
                    buf.push(c);
                }
            }
            Ok(py_str(&buf))
        }
        // Tuple
        '(' => {
            *pos += 1;
            let mut items = Vec::new();
            skip_ws(chars, pos);
            if *pos < chars.len() && chars[*pos] == ')' {
                *pos += 1;
                return Ok(py_tuple(items));
            }
            loop {
                skip_ws(chars, pos);
                let item = parse_literal(chars, pos)?;
                items.push(item);
                skip_ws(chars, pos);
                if *pos >= chars.len() {
                    return Err(PyError::ValueError(
                        "malformed node or string: unterminated tuple".to_string(),
                    ));
                }
                if chars[*pos] == ')' {
                    *pos += 1;
                    break;
                }
                if chars[*pos] != ',' {
                    return Err(PyError::ValueError(
                        "malformed node or string: expected ',' or ')' in tuple".to_string(),
                    ));
                }
                *pos += 1;
            }
            Ok(py_tuple(items))
        }
        // List
        '[' => {
            *pos += 1;
            let mut items = Vec::new();
            skip_ws(chars, pos);
            if *pos < chars.len() && chars[*pos] == ']' {
                *pos += 1;
                return Ok(py_list(items));
            }
            loop {
                skip_ws(chars, pos);
                let item = parse_literal(chars, pos)?;
                items.push(item);
                skip_ws(chars, pos);
                if *pos >= chars.len() {
                    return Err(PyError::ValueError(
                        "malformed node or string: unterminated list".to_string(),
                    ));
                }
                if chars[*pos] == ']' {
                    *pos += 1;
                    break;
                }
                if chars[*pos] != ',' {
                    return Err(PyError::ValueError(
                        "malformed node or string: expected ',' or ']' in list".to_string(),
                    ));
                }
                *pos += 1;
            }
            Ok(py_list(items))
        }
        // Dict
        '{' => {
            *pos += 1;
            let dict_obj = py_dict();
            skip_ws(chars, pos);
            if *pos < chars.len() && chars[*pos] == '}' {
                *pos += 1;
                return Ok(dict_obj);
            }
            loop {
                skip_ws(chars, pos);
                let key = parse_literal(chars, pos)?;
                skip_ws(chars, pos);
                if *pos >= chars.len() || chars[*pos] != ':' {
                    return Err(PyError::ValueError(
                        "malformed node or string: expected ':' in dict literal".to_string(),
                    ));
                }
                *pos += 1;
                skip_ws(chars, pos);
                let value = parse_literal(chars, pos)?;
                // Set key-value in dict object
                let key_str = key.str();
                if let PyObject::Dict(ref mut d) = *dict_obj.borrow_mut() {
                    d.set(py_str(&key_str), value).ok();
                }
                skip_ws(chars, pos);
                if *pos >= chars.len() {
                    return Err(PyError::ValueError(
                        "malformed node or string: unterminated dict".to_string(),
                    ));
                }
                if chars[*pos] == '}' {
                    *pos += 1;
                    break;
                }
                if chars[*pos] != ',' {
                    return Err(PyError::ValueError(
                        "malformed node or string: expected ',' or '}' in dict".to_string(),
                    ));
                }
                *pos += 1;
            }
            Ok(dict_obj)
        }
        // Number or keyword literal
        _ => {
            let _start = *pos;
            let mut buf = String::new();
            // Accumulate identifier-like or number characters
            while *pos < chars.len() {
                let c = chars[*pos];
                if c.is_alphanumeric() || c == '.' || c == '_' || c == '-' || c == '+' {
                    // For negative/positive numbers, handle the sign
                    if (c == '-' || c == '+') && !buf.is_empty() && buf != "-" && buf != "+" {
                        // Signs allowed only at the start or after 'e'/'E'
                        if buf.ends_with('e') || buf.ends_with('E') {
                            buf.push(c);
                            *pos += 1;
                        } else {
                            break;
                        }
                    } else {
                        buf.push(c);
                        *pos += 1;
                    }
                } else {
                    break;
                }
            }
            if buf.is_empty() {
                return Err(PyError::ValueError(format!(
                    "malformed node or string: unexpected character '{}' at position {}",
                    chars[*pos], *pos
                )));
            }
            // Check keywords
            match buf.as_str() {
                "True" => return Ok(py_bool(true)),
                "False" => return Ok(py_bool(false)),
                "None" => return Ok(py_none()),
                _ => {}
            }
            // Check for float (contains '.')
            if buf.contains('.') || buf.contains('e') || buf.contains('E') {
                match buf.parse::<f64>() {
                    Ok(v) => Ok(py_float(v)),
                    Err(_) => Err(PyError::ValueError(format!(
                        "malformed node or string: invalid float literal '{}'",
                        buf
                    ))),
                }
            } else {
                // Integer
                let clean = buf.replace('_', "");
                if clean.starts_with("0x") || clean.starts_with("0X") {
                    match i64::from_str_radix(&clean[2..], 16) {
                        Ok(v) => Ok(py_int(v)),
                        Err(_) => Err(PyError::ValueError(format!(
                            "malformed node or string: invalid hex literal '{}'",
                            buf
                        ))),
                    }
                } else if clean.starts_with("0o") || clean.starts_with("0O") {
                    match i64::from_str_radix(&clean[2..], 8) {
                        Ok(v) => Ok(py_int(v)),
                        Err(_) => Err(PyError::ValueError(format!(
                            "malformed node or string: invalid octal literal '{}'",
                            buf
                        ))),
                    }
                } else if clean.starts_with("0b") || clean.starts_with("0B") {
                    match i64::from_str_radix(&clean[2..], 2) {
                        Ok(v) => Ok(py_int(v)),
                        Err(_) => Err(PyError::ValueError(format!(
                            "malformed node or string: invalid binary literal '{}'",
                            buf
                        ))),
                    }
                } else {
                    match clean.parse::<i64>() {
                        Ok(v) => Ok(py_int(v)),
                        Err(_) => Err(PyError::ValueError(format!(
                            "malformed node or string: invalid integer literal '{}'",
                            buf
                        ))),
                    }
                }
            }
        }
    }
}
