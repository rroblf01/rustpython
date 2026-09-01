use crate::object::*;
use std::cell::RefCell;
use std::collections::HashMap as StdHashMap;

use super::error::dialect_error;
use super::typename::type_name_of;

#[derive(Clone, Debug)]
pub(crate) struct CsvDialect {
    pub delimiter: char,
    pub quotechar: Option<char>,
    pub escapechar: Option<char>,
    pub doublequote: bool,
    pub skipinitialspace: bool,
    pub lineterminator: String,
    pub quoting: i64,
    pub strict: bool,
}
impl CsvDialect {
    pub fn excel() -> Self { Self { delimiter: ',', quotechar: Some('"'), escapechar: None, doublequote: true, skipinitialspace: false, lineterminator: "\r\n".to_string(), quoting: 0, strict: false } }
    pub fn excel_tab() -> Self { Self { delimiter: '\t', quotechar: Some('"'), escapechar: None, doublequote: true, skipinitialspace: false, lineterminator: "\r\n".to_string(), quoting: 0, strict: false } }
    pub fn unix() -> Self { Self { delimiter: ',', quotechar: Some('"'), escapechar: None, doublequote: true, skipinitialspace: false, lineterminator: "\n".to_string(), quoting: 1, strict: false } }
}

thread_local! {
    pub(crate) static CSV_FIELD_LIMIT: RefCell<usize> = RefCell::new(128*1024);
    pub(crate) static CSV_DIALECTS: RefCell<StdHashMap<String, CsvDialect>> = RefCell::new({
        let mut m = StdHashMap::new();
        m.insert("excel".to_string(), CsvDialect::excel());
        m.insert("excel-tab".to_string(), CsvDialect::excel_tab());
        m.insert("unix".to_string(), CsvDialect::unix());
        m
    });
}

pub(crate) fn build_dialect(dialect_arg: Option<PyObjectRef>, kwargs: &StdHashMap<String, PyObjectRef>) -> PyResult<CsvDialect> {
        let mut base = CsvDialect::excel();
        let dialect_is_none = dialect_arg.is_none();
        if let Some(darg) = dialect_arg.clone() {
            let is_str = matches!(&*darg.borrow(), PyObject::Str(_));
            let is_none = matches!(&*darg.borrow(), PyObject::None);
            if is_str {
                let s = darg.str();
                let name = s.to_string();
                if let Some(found) = CSV_DIALECTS.with(|c| c.borrow().get(name.as_str()).cloned()) {
                    base = found;
                } else {
                    return Err(PyError::Exception("Error".to_string(), PyObjectRef::new(PyObject::Exception{ typ: "Error".to_string(), args: vec![py_str(&format!("unknown dialect {}", name))], cause: None, suppress_context: false, context: None, traceback: None, extra: None })));
                }
            } else if is_none {
            } else {
                let obj = darg.clone();
                    if let Ok(v) = obj.borrow().get_attribute("delimiter") {
                        let vb = v.borrow();
                        if let PyObject::Str(s) = &*vb { if s.chars().count()==1 { base.delimiter = s.chars().next().unwrap(); } }
                    }
                    if let Ok(v) = obj.borrow().get_attribute("quotechar") {
                        let vb = v.borrow();
                        if let PyObject::Str(s) = &*vb { if s.is_empty() { base.quotechar = None; } else if s.chars().count()==1 { base.quotechar = Some(s.chars().next().unwrap()); } }
                        else if matches!(&*vb, PyObject::None) { base.quotechar = None; }
                    }
                    if let Ok(v) = obj.borrow().get_attribute("escapechar") {
                        let vb = v.borrow();
                        if let PyObject::Str(s) = &*vb { if s.is_empty() { base.escapechar = None; } else if s.chars().count()==1 { base.escapechar = Some(s.chars().next().unwrap()); } }
                        else if matches!(&*vb, PyObject::None) { base.escapechar = None; }
                    }
                    if let Ok(v) = obj.borrow().get_attribute("doublequote") { base.doublequote = v.truthy(); }
                    if let Ok(v) = obj.borrow().get_attribute("skipinitialspace") { base.skipinitialspace = v.truthy(); }
                    if let Ok(v) = obj.borrow().get_attribute("lineterminator") { base.lineterminator = v.str(); }
                    if let Ok(v) = obj.borrow().get_attribute("quoting") { if let Some(n) = v.as_i64() { base.quoting = n; } }
                    if let Ok(v) = obj.borrow().get_attribute("strict") { base.strict = v.truthy(); };
            }
        }
        if dialect_is_none {
            if let Some(dval) = kwargs.get("dialect") {
                let is_str = matches!(&*dval.borrow(), PyObject::Str(_));
                if is_str {
                    let s = dval.str();
                    if let Some(found) = CSV_DIALECTS.with(|c| c.borrow().get(s.as_str()).cloned()) {
                        base = found;
                    } else {
                        return Err(PyError::Exception("Error".to_string(), PyObjectRef::new(PyObject::Exception{ typ: "Error".to_string(), args: vec![py_str(&format!("unknown dialect {}", s))], cause: None, suppress_context: false, context: None, traceback: None, extra: None })));
                    }
                } else {
                    let obj = dval.clone();
                    if let Ok(v) = obj.borrow().get_attribute("delimiter") {
                        let vb = v.borrow();
                        if let PyObject::Str(s) = &*vb { if s.chars().count()==1 { base.delimiter = s.chars().next().unwrap(); } }
                    }
                    if let Ok(v) = obj.borrow().get_attribute("quotechar") {
                        let vb = v.borrow();
                        if let PyObject::Str(s) = &*vb { if s.is_empty() { base.quotechar = None; } else if s.chars().count()==1 { base.quotechar = Some(s.chars().next().unwrap()); } }
                        else if matches!(&*vb, PyObject::None) { base.quotechar = None; }
                    }
                    if let Ok(v) = obj.borrow().get_attribute("escapechar") {
                        let vb = v.borrow();
                        if let PyObject::Str(s) = &*vb { if s.is_empty() { base.escapechar = None; } else if s.chars().count()==1 { base.escapechar = Some(s.chars().next().unwrap()); } }
                        else if matches!(&*vb, PyObject::None) { base.escapechar = None; }
                    }
                    if let Ok(v) = obj.borrow().get_attribute("doublequote") { base.doublequote = v.truthy(); }
                    if let Ok(v) = obj.borrow().get_attribute("skipinitialspace") { base.skipinitialspace = v.truthy(); }
                    if let Ok(v) = obj.borrow().get_attribute("lineterminator") { base.lineterminator = v.str(); }
                    if let Ok(v) = obj.borrow().get_attribute("quoting") { if let Some(n) = v.as_i64() { base.quoting = n; } }
                    if let Ok(v) = obj.borrow().get_attribute("strict") { base.strict = v.truthy(); };
                }
            }
        }
        for (k,v) in kwargs.iter() {
            if k=="dialect" { continue; }
            match k.as_str() {
                "delimiter" => {
                    let vb = v.borrow();
                    if let PyObject::Str(s) = &*vb {
                        if s.chars().count()!=1 { return Err(PyError::type_error("delimiter must be a 1-character string")); }
                        let ch = s.chars().next().unwrap();
                        if ch=='\n' || ch=='\r' { return Err(PyError::value_error("delimiter must not be \\n or \\r")); }
                        base.delimiter = ch;
                    } else if matches!(&*vb, PyObject::None) {
                        return Err(PyError::type_error("delimiter must be set"));
                    } else {
                        return Err(PyError::type_error("delimiter must be string"));
                    }
                },
                "quotechar" => {
                    let vb = v.borrow();
                    if let PyObject::Str(s) = &*vb {
                        if s.is_empty() { return Err(PyError::type_error("quotechar must be a 1-character string")); }
                        else if s.chars().count()!=1 { return Err(PyError::type_error("quotechar must be a 1-character string")); }
                        else {
                            let ch = s.chars().next().unwrap();
                            if ch=='\n' || ch=='\r' { return Err(PyError::value_error("quotechar must not be \\n or \\r")); }
                            base.quotechar = Some(ch);
                        }
                    } else if matches!(&*vb, PyObject::None) {
                        base.quotechar = None;
                    } else {
                        return Err(PyError::type_error("quotechar must be string"));
                    }
                },
                "escapechar" => {
                    let vb = v.borrow();
                    if let PyObject::Str(s) = &*vb {
                        if s.is_empty() { return Err(PyError::type_error("escapechar must be a 1-character string")); }
                        else if s.chars().count()!=1 { return Err(PyError::type_error("escapechar must be a 1-character string")); }
                        else {
                            let ch = s.chars().next().unwrap();
                            if ch=='\n' || ch=='\r' { return Err(PyError::value_error("escapechar must not be \\n or \\r")); }
                            base.escapechar = Some(ch);
                        }
                    } else if matches!(&*vb, PyObject::None) {
                        base.escapechar = None;
                    } else {
                        return Err(PyError::type_error("escapechar must be string"));
                    }
                },
                "doublequote" => { base.doublequote = v.truthy(); },
                "skipinitialspace" => { base.skipinitialspace = v.truthy(); },
                "lineterminator" => {
                    let vb = v.borrow();
                    if let PyObject::Str(s) = &*vb { base.lineterminator = s.to_string(); } else { return Err(PyError::type_error("lineterminator must be string")); }
                },
                "quoting" => {
                    if let Some(n) = v.as_i64() { base.quoting = n; } else { return Err(PyError::type_error("quoting must be int")); }
                    if (base.quoting == 1 || base.quoting == 2 || base.quoting == 4 || base.quoting == 5) && base.quotechar.is_none() {
                        return Err(PyError::type_error("quotechar must be set if quoting is not QUOTE_NONE"));
                    }
                },
                "strict" => { base.strict = v.truthy(); },
                _ => { return Err(PyError::type_error(format!("unexpected keyword argument '{}'", k))); }
            }
        }
        if base.delimiter=='\n' || base.delimiter=='\r' { return Err(PyError::value_error("delimiter must not be \\n or \\r")); }
        if let Some(q) = base.quotechar { if q=='\n' || q=='\r' { return Err(PyError::value_error("quotechar must not be \\n or \\r")); } }
        if let Some(e) = base.escapechar { if e=='\n' || e=='\r' { return Err(PyError::value_error("escapechar must not be \\n or \\r")); } }
        if let Some(q) = base.quotechar {
            if base.delimiter==q { return Err(PyError::value_error("delimiter and quotechar must be different")); }
            if let Some(e) = base.escapechar { if e==q { return Err(PyError::value_error("escapechar and quotechar must be different")); } }
        }
        if let Some(e) = base.escapechar { if base.delimiter==e { return Err(PyError::value_error("delimiter and escapechar must be different")); } }
        if base.skipinitialspace {
            if let Some(e) = base.escapechar { if e==' ' { return Err(PyError::value_error("escapechar must not be space with skipinitialspace")); } }
            if let Some(q) = base.quotechar { if q==' ' { return Err(PyError::value_error("quotechar must not be space with skipinitialspace")); } }
        }
        if (base.quoting == 1 || base.quoting == 2 || base.quoting == 4 || base.quoting == 5) && base.quotechar.is_none() {
            return Err(PyError::type_error("quotechar must be set if quoting is not QUOTE_NONE"));
        }
        if base.lineterminator.contains(base.delimiter) { return Err(PyError::value_error("lineterminator must not contain delimiter")); }
        if let Some(q) = base.quotechar { if base.lineterminator.contains(q) { return Err(PyError::value_error("lineterminator must not contain quotechar")); } }
        if let Some(e) = base.escapechar { if base.lineterminator.contains(e) { return Err(PyError::value_error("lineterminator must not contain escapechar")); } }
        Ok(base)
    }

pub(crate) fn extract_kwargs(args: &[PyObjectRef], expected_positional: usize) -> (Option<PyObjectRef>, StdHashMap<String, PyObjectRef>) {
        let mut dialect_arg: Option<PyObjectRef> = None;
        let mut kwargs: StdHashMap<String, PyObjectRef> = StdHashMap::new();
        if args.len() > expected_positional {
            if args.len() == expected_positional + 1 {
                let last = &args[expected_positional];
                if let PyObject::Dict(d) = &*last.borrow() {
                    for (k,v) in d.items() { kwargs.insert(k.str(), v); }
                    return (None, kwargs);
                } else {
                    dialect_arg = Some(last.clone());
                    return (dialect_arg, kwargs);
                }
            } else {
                dialect_arg = Some(args[expected_positional].clone());
                let last = args.last().unwrap();
                if let PyObject::Dict(d) = &*last.borrow() {
                    for (k,v) in d.items() { kwargs.insert(k.str(), v); }
                }
                return (dialect_arg, kwargs);
            }
        }
        (dialect_arg, kwargs)
    }

pub(crate) fn validate_dialect_obj(obj: &PyObjectRef) -> PyResult<()> {
        let get = |name: &str| -> Option<PyObjectRef> { obj.borrow().get_attribute(name).ok() };
        // delimiter
        let delim_val = get("delimiter").unwrap_or_else(py_none);
        // quotechar, escapechar, lineterminator, quoting, etc.
        let quote_val = get("quotechar").unwrap_or_else(py_none);
        let escape_val = get("escapechar").unwrap_or_else(py_none);
        let lineterm_val = get("lineterminator").unwrap_or_else(py_none);
        let quoting_val = get("quoting").unwrap_or_else(py_none);
        // delimiter must be unicode char
        {
            let b = delim_val.borrow();
            match &*b {
                PyObject::Str(s) => {
                    let cnt = s.chars().count();
                    if cnt != 1 {
                        return Err(dialect_error(format!("\"delimiter\" must be a unicode character, not a string of length {}", cnt)));
                    }
                    let ch = s.chars().next().unwrap();
                    if ch == '\n' || ch == '\r' {
                        return Err(PyError::value_error(format!("delimiter must not be \\n or \\r")));
                    }
                },
                PyObject::Bytes(_) => return Err(dialect_error("\"delimiter\" must be a unicode character, not bytes".to_string())),
                PyObject::None => return Err(dialect_error("\"delimiter\" must be a unicode character, not NoneType".to_string())),
                PyObject::Int(_) => return Err(dialect_error("\"delimiter\" must be a unicode character, not int".to_string())),
                _ => return Err(dialect_error(format!("\"delimiter\" must be a unicode character, not {}", type_name_of(&delim_val)))),
            }
        }
        // quotechar must be unicode char or None
        {
            let b = quote_val.borrow();
            if !matches!(&*b, PyObject::None) {
                match &*b {
                    PyObject::Str(s) => {
                        let cnt = s.chars().count();
                        if cnt != 1 {
                            return Err(dialect_error(format!("\"quotechar\" must be a unicode character or None, not a string of length {}", cnt)));
                        }
                        let ch = s.chars().next().unwrap();
                        if ch == '\n' || ch == '\r' {
                            return Err(PyError::value_error("quotechar must not be \\n or \\r".to_string()));
                        }
                    },
                    PyObject::Bytes(_) => return Err(dialect_error("\"quotechar\" must be a unicode character or None, not bytes".to_string())),
                    PyObject::Int(_) => return Err(dialect_error("\"quotechar\" must be a unicode character or None, not int".to_string())),
                    _ => return Err(dialect_error(format!("\"quotechar\" must be a unicode character or None, not {}", type_name_of(&quote_val)))),
                }
            }
        }
        // escapechar must be unicode char or None
        {
            let b = escape_val.borrow();
            if !matches!(&*b, PyObject::None) {
                match &*b {
                    PyObject::Str(s) => {
                        let cnt = s.chars().count();
                        if cnt != 1 {
                            return Err(dialect_error(format!("\"escapechar\" must be a unicode character or None, not a string of length {}", cnt)));
                        }
                        let ch = s.chars().next().unwrap();
                        if ch == '\n' || ch == '\r' {
                            return Err(PyError::value_error("escapechar must not be \\n or \\r".to_string()));
                        }
                    },
                    PyObject::Bytes(_) => return Err(dialect_error("\"escapechar\" must be a unicode character or None, not bytes".to_string())),
                    PyObject::Int(_) => return Err(dialect_error("\"escapechar\" must be a unicode character or None, not int".to_string())),
                    _ => return Err(dialect_error(format!("\"escapechar\" must be a unicode character or None, not {}", type_name_of(&escape_val)))),
                }
            }
        }
        // lineterminator must be string
        {
            let b = lineterm_val.borrow();
            match &*b {
                PyObject::Str(_) => {},
                PyObject::None => return Err(dialect_error("\"lineterminator\" must be a string, not NoneType".to_string())),
                PyObject::Int(_) => return Err(dialect_error("\"lineterminator\" must be a string, not int".to_string())),
                PyObject::Bytes(_) => return Err(dialect_error("\"lineterminator\" must be a string, not bytes".to_string())),
                _ => return Err(dialect_error(format!("\"lineterminator\" must be a string, not {}", type_name_of(&lineterm_val)))),
            }
        }
        // quoting must be int 0-5
        {
            let b = quoting_val.borrow();
            if let Some(n) = quoting_val.as_i64() {
                if n < 0 || n > 5 {
                    return Err(dialect_error("bad \"quoting\" value".to_string()));
                }
            } else {
                // quoting is None or not int
                if matches!(&*b, PyObject::None) {
                    return Err(dialect_error("bad \"quoting\" value".to_string()));
                }
                return Err(dialect_error(format!("\"quoting\" must be an integer, not {}", type_name_of(&quoting_val))));
            }
        }
        // check invalid chars value errors: \n, \r etc already, also delimiter/quote/escape distinct and not space with skipinitialspace
        // get chars for further checks
        let delim_ch = {
            let b = delim_val.borrow();
            if let PyObject::Str(s) = &*b { s.chars().next().unwrap() } else { ',' }
        };
        let quote_ch_opt = {
            let b = quote_val.borrow();
            if let PyObject::Str(s) = &*b { if s.is_empty() { None } else { Some(s.chars().next().unwrap()) } } else { None }
        };
        let escape_ch_opt = {
            let b = escape_val.borrow();
            if let PyObject::Str(s) = &*b { if s.is_empty() { None } else { Some(s.chars().next().unwrap()) } } else { None }
        };
        let lineterm_str: String = {
            let b = lineterm_val.borrow();
            if let PyObject::Str(s) = &*b { s.to_string() } else { "\r\n".to_string() }
        };
        // delimiter and quotechar must be different
        if let Some(q) = quote_ch_opt {
            if delim_ch == q { return Err(PyError::value_error("delimiter and quotechar must be different")); }
            if let Some(e) = escape_ch_opt { if e == q { return Err(PyError::value_error("escapechar and quotechar must be different")); } }
        }
        if let Some(e) = escape_ch_opt { if delim_ch == e { return Err(PyError::value_error("delimiter and escapechar must be different")); } }
        // skipinitialspace checks
        let skip = get("skipinitialspace").map(|v| v.truthy()).unwrap_or(false);
        if skip {
            if let Some(e) = escape_ch_opt { if e == ' ' { return Err(PyError::value_error("escapechar must not be space with skipinitialspace")); } }
            if let Some(q) = quote_ch_opt { if q == ' ' { return Err(PyError::value_error("quotechar must not be space with skipinitialspace")); } }
        }
        // lineterminator must not contain delimiter etc
        if lineterm_str.contains(delim_ch) { return Err(PyError::value_error("lineterminator must not contain delimiter")); }
        if let Some(q) = quote_ch_opt { if lineterm_str.contains(q) { return Err(PyError::value_error("lineterminator must not contain quotechar")); } }
        if let Some(e) = escape_ch_opt { if lineterm_str.contains(e) { return Err(PyError::value_error("lineterminator must not contain escapechar")); } }
        // quoting requires quotechar if not QUOTE_NONE and not QUOTE_MINIMAL
        let quoting_n = quoting_val.as_i64().unwrap_or(0);
        if (quoting_n == 1 || quoting_n == 2 || quoting_n == 4 || quoting_n == 5) && quote_ch_opt.is_none() {
            return Err(PyError::type_error("quotechar must be set if quoting is not QUOTE_NONE"));
        }
        // invalid chars for delimiter/quote/escape already handled for \n \r, but also need to handle \n \r as ValueError for those fields when set to that
        // Already returned value_error for those, but also need to check for ' ' with skipinitialspace? already.
        Ok(())
    }

pub(crate) fn collect_lines(iterable: &PyObjectRef) -> PyResult<Vec<String>> {
        let actual = if let Ok(inner) = iterable.borrow().get_attribute("_file") {
            inner
        } else {
            iterable.clone()
        };
        let read_try = crate::object::with_vm_mut(|vm| crate::object::call_method_rebound(vm, &actual, "read", vec![]));
        if let Ok(Ok(v)) = read_try {
            if matches!(&*v.borrow(), PyObject::Str(_)) {
                let s = v.str();
                return Ok(s.lines().map(|l| {
                    let mut t = l.to_string();
                    while t.ends_with('\r') || t.ends_with('\n') { t.pop(); }
                    t
                }).collect());
            } else if matches!(&*v.borrow(), PyObject::Bytes(_)) {
                return Err(PyError::Exception("Error".to_string(), PyObjectRef::new(PyObject::Exception{ typ: "Error".to_string(), args: vec![py_str("iterator should return strings, not bytes")], cause: None, suppress_context: false, context: None, traceback: None, extra: None })));
            }
        }
        let try_iter = crate::object::with_vm_mut(|vm| crate::object::call_method_rebound(vm, &actual, "__iter__", vec![]));
        let iter = if let Ok(Ok(it)) = try_iter { it } else { crate::object::builtin_iter(&[actual.clone()])? };
        let mut lines = Vec::new();
        loop {
            match crate::object::builtin_next(&[iter.clone()]) {
                Ok(val) => {
                    if matches!(&*val.borrow(), PyObject::Bytes(_)) {
                        return Err(PyError::Exception("Error".to_string(), PyObjectRef::new(PyObject::Exception{ typ: "Error".to_string(), args: vec![py_str("iterator should return strings, not bytes")], cause: None, suppress_context: false, context: None, traceback: None, extra: None })));
                    }
                    let mut s = val.str();
                    while s.ends_with('\r') || s.ends_with('\n') {
                        s.pop();
                    }
                    lines.push(s);
                },
                Err(PyError::StopIteration) => break,
                Err(e) => return Err(e),
            }
        }
        Ok(lines)
    }

pub(crate) fn parse_csv_row(s: &str, dialect: &CsvDialect) -> PyResult<Vec<PyObjectRef>> {
        let mut fields: Vec<String> = Vec::new();
        let mut field = String::new();
        let mut in_quotes = false;
        let mut chars = s.chars().peekable();
        if dialect.skipinitialspace {
            while chars.peek() == Some(&' ') { chars.next(); }
            if chars.peek().is_none() {
                // line was all spaces
                if dialect.delimiter == ' ' {
                    return Ok(vec![py_str("")]);
                } else {
                    return Ok(vec![py_str("")]);
                }
            }
        }
        let quotechar = if dialect.quoting == 3 { None } else { dialect.quotechar };
        let escapechar = dialect.escapechar;
        let delimiter = dialect.delimiter;
        while let Some(c) = chars.next() {
            if in_quotes {
                if Some(c) == escapechar {
                    if let Some(nxt) = chars.next() {
                        field.push(nxt);
                    } else {
                        // escape at end of line -> treat as newline if not strict
                        if dialect.strict {
                            return Err(PyError::Exception("Error".to_string(), PyObjectRef::new(PyObject::Exception{ typ: "Error".to_string(), args: vec![py_str("unexpected end of data")], cause: None, suppress_context: false, context: None, traceback: None, extra: None })));
                        } else {
                            field.push('\n');
                        }
                    }
                } else if Some(c) == quotechar {
                    if dialect.doublequote && chars.peek() == Some(&c) {
                        chars.next();
                        field.push(c);
                    } else {
                        in_quotes = false;
                        if let Some(&next) = chars.peek() {
                            if next != delimiter && next != '\r' && next != '\n' {
                                if dialect.strict {
                                    return Err(PyError::Exception("Error".to_string(), PyObjectRef::new(PyObject::Exception{ typ: "Error".to_string(), args: vec![py_str(&format!("'{}' expected after '\"'", delimiter))], cause: None, suppress_context: false, context: None, traceback: None, extra: None })));
                                }
                            }
                        }
                    }
                } else {
                    field.push(c);
                }
            } else {
                if Some(c) == escapechar {
                    if let Some(nxt) = chars.next() {
                        field.push(nxt);
                    } else {
                        if dialect.strict {
                            return Err(PyError::Exception("Error".to_string(), PyObjectRef::new(PyObject::Exception{ typ: "Error".to_string(), args: vec![py_str("unexpected end of data")], cause: None, suppress_context: false, context: None, traceback: None, extra: None })));
                        } else {
                            field.push('\n');
                        }
                    }
                } else if Some(c) == quotechar {
                    if field.is_empty() {
                        in_quotes = true;
                    } else {
                        if dialect.strict {
                            return Err(PyError::Exception("Error".to_string(), PyObjectRef::new(PyObject::Exception{ typ: "Error".to_string(), args: vec![py_str("unexpected quote")], cause: None, suppress_context: false, context: None, traceback: None, extra: None })));
                        }
                        field.push(c);
                    }
                } else if c == delimiter {
                    fields.push(field);
                    field = String::new();
                    if dialect.skipinitialspace {
                        while chars.peek() == Some(&' ') { chars.next(); }
                    }
                } else if c == '\r' || c == '\n' {
                    if !in_quotes {
                        return Err(PyError::Exception("Error".to_string(), PyObjectRef::new(PyObject::Exception{ typ: "Error".to_string(), args: vec![py_str("with newline=''")], cause: None, suppress_context: false, context: None, traceback: None, extra: None })));
                    }
                    field.push(c);
                } else {
                    field.push(c);
                }
            }
        }
        if in_quotes {
            if dialect.strict {
                return Err(PyError::Exception("Error".to_string(), PyObjectRef::new(PyObject::Exception{ typ: "Error".to_string(), args: vec![py_str("unexpected end of data")], cause: None, suppress_context: false, context: None, traceback: None, extra: None })));
            }
        }
        fields.push(field);
        let limit = CSV_FIELD_LIMIT.with(|c| *c.borrow());
        for f in &fields {
            if f.len() > limit {
                return Err(PyError::Exception("Error".to_string(), PyObjectRef::new(PyObject::Exception{ typ: "Error".to_string(), args: vec![py_str("field larger than field limit")], cause: None, suppress_context: false, context: None, traceback: None, extra: None })));
            }
        }
        let mut out: Vec<PyObjectRef> = Vec::new();
        for f in fields {
            let converted = if dialect.quoting == 2 {
                if f.is_empty() { py_str(&f) } else {
                    match f.parse::<f64>() { Ok(num) => py_float(num), Err(_) => py_str(&f) }
                }
            } else if dialect.quoting == 5 {
                if f.is_empty() { py_none() } else { py_str(&f) }
            } else if dialect.quoting == 4 {
                if f.is_empty() { py_none() } else {
                    match f.parse::<f64>() { Ok(num) => py_float(num), Err(_) => py_str(&f) }
                }
            } else {
                py_str(&f)
            };
            out.push(converted);
        }
        Ok(out)
    }

pub(crate) fn register_dialect_types(d: &mut StdHashMap<String, PyObjectRef>) {
    let dialect_base_type = PyObjectRef::new(PyObject::Type{ name: "Dialect".to_string(), dict: Box::new(str_map_to_typedict({
        let mut m = StdHashMap::new();
        m.insert_str("__init__", PyObjectRef::new(PyObject::BuiltinFunction{ name: "__init__".to_string(), func: |args| {
            if args.is_empty() { return Ok(py_none()); }
            let self_obj = &args[0];
            validate_dialect_obj(self_obj)?;
            Ok(py_none())
        }}));
        m
    })), bases: vec![], mro: vec![] });
    d.insert("Dialect".to_string(), dialect_base_type.clone());
    let excel_type = PyObjectRef::new(PyObject::Type{ name: "excel".to_string(), dict: Box::new(str_map_to_typedict({
        let mut m = StdHashMap::new();
        m.insert_str("delimiter", py_str(","));
        m.insert_str("quotechar", py_str("\""));
        m.insert_str("escapechar", py_none());
        m.insert_str("doublequote", py_bool(true));
        m.insert_str("skipinitialspace", py_bool(false));
        m.insert_str("lineterminator", py_str("\r\n"));
        m.insert_str("quoting", py_int(0));
        m.insert_str("strict", py_bool(false));
        m
    })), bases: vec![dialect_base_type.clone()], mro: vec![] });
    d.insert("excel".to_string(), excel_type.clone());
    let excel_tab_type = PyObjectRef::new(PyObject::Type{ name: "excel_tab".to_string(), dict: Box::new(str_map_to_typedict({
        let mut m = StdHashMap::new();
        m.insert_str("delimiter", py_str("\t"));
        m.insert_str("quotechar", py_str("\""));
        m.insert_str("escapechar", py_none());
        m.insert_str("doublequote", py_bool(true));
        m.insert_str("skipinitialspace", py_bool(false));
        m.insert_str("lineterminator", py_str("\r\n"));
        m.insert_str("quoting", py_int(0));
        m.insert_str("strict", py_bool(false));
        m
    })), bases: vec![excel_type.clone()], mro: vec![] });
    d.insert("excel_tab".to_string(), excel_tab_type);
    let unix_type = PyObjectRef::new(PyObject::Type{ name: "unix_dialect".to_string(), dict: Box::new(str_map_to_typedict({
        let mut m = StdHashMap::new();
        m.insert_str("delimiter", py_str(","));
        m.insert_str("quotechar", py_str("\""));
        m.insert_str("escapechar", py_none());
        m.insert_str("doublequote", py_bool(true));
        m.insert_str("skipinitialspace", py_bool(false));
        m.insert_str("lineterminator", py_str("\n"));
        m.insert_str("quoting", py_int(1));
        m.insert_str("strict", py_bool(false));
        m
    })), bases: vec![dialect_base_type.clone()], mro: vec![] });
    d.insert("unix_dialect".to_string(), unix_type.clone());
}
