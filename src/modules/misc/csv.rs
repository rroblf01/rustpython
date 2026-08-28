use crate::object::*;
use std::collections::HashMap;
use std::rc::Rc;
#[allow(unused_imports)]
use std::cell::RefCell;

mod error;
use error::dialect_error;
mod typename;
use typename::type_name_of;

pub fn create_csv_dict() -> HashMap<String, PyObjectRef> {
    use std::cell::RefCell;
    use std::collections::HashMap as StdHashMap;

    thread_local! {
        static CSV_FIELD_LIMIT: RefCell<usize> = RefCell::new(128*1024);
        static CSV_DIALECTS: RefCell<StdHashMap<String, CsvDialect>> = RefCell::new({
            let mut m = StdHashMap::new();
            m.insert("excel".to_string(), CsvDialect::excel());
            m.insert("excel-tab".to_string(), CsvDialect::excel_tab());
            m.insert("unix".to_string(), CsvDialect::unix());
            m
        });
    }

    #[derive(Clone, Debug)]
    struct CsvDialect {
        delimiter: char,
        quotechar: Option<char>,
        escapechar: Option<char>,
        doublequote: bool,
        skipinitialspace: bool,
        lineterminator: String,
        quoting: i64,
        strict: bool,
    }
    impl CsvDialect {
        fn excel() -> Self { Self { delimiter: ',', quotechar: Some('"'), escapechar: None, doublequote: true, skipinitialspace: false, lineterminator: "\r\n".to_string(), quoting: 0, strict: false } }
        fn excel_tab() -> Self { Self { delimiter: '\t', quotechar: Some('"'), escapechar: None, doublequote: true, skipinitialspace: false, lineterminator: "\r\n".to_string(), quoting: 0, strict: false } }
        fn unix() -> Self { Self { delimiter: ',', quotechar: Some('"'), escapechar: None, doublequote: true, skipinitialspace: false, lineterminator: "\n".to_string(), quoting: 1, strict: false } }
    }

    fn build_dialect(dialect_arg: Option<PyObjectRef>, kwargs: &StdHashMap<String, PyObjectRef>) -> PyResult<CsvDialect> {
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

    fn extract_kwargs(args: &[PyObjectRef], expected_positional: usize) -> (Option<PyObjectRef>, StdHashMap<String, PyObjectRef>) {
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

    fn collect_lines(iterable: &PyObjectRef) -> PyResult<Vec<String>> {
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

    fn parse_csv_row(s: &str, dialect: &CsvDialect) -> PyResult<Vec<PyObjectRef>> {
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

    fn parse_csv_lines(lines: Vec<String>, dialect: &CsvDialect) -> PyResult<Vec<Vec<PyObjectRef>>> {
        if lines.is_empty() {
            return Ok(vec![]);
        }
        // Handle special case: single empty string => [[]]
        if lines.len() == 1 && lines[0].is_empty() {
            return Ok(vec![vec![]]);
        }
        let mut rows: Vec<Vec<PyObjectRef>> = Vec::new();
        let mut cur_row: Vec<String> = Vec::new();
        let mut cur_field = String::new();
        let mut in_quotes = false;
        let quotechar = if dialect.quoting == 3 { None } else { dialect.quotechar };
        let escapechar = dialect.escapechar;
        let delimiter = dialect.delimiter;
        let mut line_idx = 0;
        while line_idx < lines.len() {
            let line = &lines[line_idx];
            let mut chars = line.chars().peekable();
            // handle skipinitialspace at start of line if not in_quotes and new row
            if !in_quotes && cur_field.is_empty() && cur_row.is_empty() && dialect.skipinitialspace {
                while chars.peek() == Some(&' ') { chars.next(); }
                if chars.peek().is_none() && !in_quotes {
                    // line is all spaces or empty and skipinitialspace
                    if delimiter == ' ' {
                        rows.push(vec![py_str("")]);
                    } else {
                        rows.push(vec![py_str("")]);
                    }
                    line_idx += 1;
                    continue;
                }
            }
            let mut pos_done = false;
            while let Some(c) = chars.next() {
                if in_quotes {
                    if Some(c) == escapechar {
                        if let Some(nxt) = chars.next() { cur_field.push(nxt); } else {
                            // escape at end of line -> treat as newline? but we are within quoted field spanning lines
                            // This should be handled as newline continuation
                            // peek next line existence
                            cur_field.push('\n');
                        }
                    } else if Some(c) == quotechar {
                        if dialect.doublequote && chars.peek() == Some(&c) {
                            chars.next(); cur_field.push(c);
                        } else {
                            in_quotes = false;
                            if let Some(&next) = chars.peek() {
                                if next != delimiter {
                                    if dialect.strict {
                                        return Err(PyError::Exception("Error".to_string(), PyObjectRef::new(PyObject::Exception{ typ: "Error".to_string(), args: vec![py_str(&format!("'{}' expected after '\"'", delimiter))], cause: None, suppress_context: false, context: None, traceback: None, extra: None })));
                                    }
                                }
                            }
                        }
                    } else {
                        cur_field.push(c);
                    }
                } else {
                    if Some(c) == escapechar {
                        if let Some(nxt) = chars.next() { cur_field.push(nxt); } else {
                            if dialect.strict {
                                return Err(PyError::Exception("Error".to_string(), PyObjectRef::new(PyObject::Exception{ typ: "Error".to_string(), args: vec![py_str("unexpected end of data")], cause: None, suppress_context: false, context: None, traceback: None, extra: None })));
                            } else {
                                cur_field.push('\n');
                            }
                        }
                    } else if Some(c) == quotechar {
                        if cur_field.is_empty() {
                            in_quotes = true;
                        } else {
                            if dialect.strict {
                                return Err(PyError::Exception("Error".to_string(), PyObjectRef::new(PyObject::Exception{ typ: "Error".to_string(), args: vec![py_str("unexpected quote")], cause: None, suppress_context: false, context: None, traceback: None, extra: None })));
                            }
                            cur_field.push(c);
                        }
                    } else if c == delimiter {
                        cur_row.push(cur_field.clone()); cur_field = String::new();
                        if dialect.skipinitialspace {
                            while chars.peek() == Some(&' ') { chars.next(); }
                        }
                    } else {
                        cur_field.push(c);
                    }
                }
            }
            // end of line
            if in_quotes {
                // quoted field spans to next line: insert newline
                cur_field.push('\n');
                line_idx += 1;
                continue;
            } else {
                // not in quotes: end row
                // check for empty line case already handled? For line empty we would have cur_row empty and cur_field empty
                if line.is_empty() && cur_field.is_empty() && cur_row.is_empty() {
                    rows.push(vec![]);
                } else {
                    cur_row.push(cur_field.clone());
                    // check field size limit and quoting conversion will be done later per row
                    // convert row strings to PyObjects with quoting handling
                    let limit = CSV_FIELD_LIMIT.with(|c| *c.borrow());
                    for f in &cur_row { if f.len() > limit { return Err(PyError::Exception("Error".to_string(), PyObjectRef::new(PyObject::Exception{ typ: "Error".to_string(), args: vec![py_str("field larger than field limit")], cause: None, suppress_context: false, context: None, traceback: None, extra: None }))); } }
                    let mut out: Vec<PyObjectRef> = Vec::new();
                    for f in cur_row.clone() {
                        let conv = if dialect.quoting == 2 {
                            if f.is_empty() { py_str(&f) } else { match f.parse::<f64>() { Ok(n) => py_float(n), Err(_) => py_str(&f) } }
                        } else if dialect.quoting == 5 {
                            if f.is_empty() { py_none() } else { py_str(&f) }
                        } else if dialect.quoting == 4 {
                            if f.is_empty() { py_none() } else { match f.parse::<f64>() { Ok(n) => py_float(n), Err(_) => py_str(&f) } }
                        } else { py_str(&f) };
                        out.push(conv);
                    }
                    rows.push(out);
                }
                cur_row = Vec::new();
                cur_field = String::new();
                line_idx += 1;
            }
        }
        // if we ended while still have pending data (file not ending with newline)
        if !cur_row.is_empty() || !cur_field.is_empty() || in_quotes {
            if !cur_row.is_empty() || !cur_field.is_empty() {
                cur_row.push(cur_field);
                let limit = CSV_FIELD_LIMIT.with(|c| *c.borrow());
                for f in &cur_row { if f.len() > limit { return Err(PyError::Exception("Error".to_string(), PyObjectRef::new(PyObject::Exception{ typ: "Error".to_string(), args: vec![py_str("field larger than field limit")], cause: None, suppress_context: false, context: None, traceback: None, extra: None }))); } }
                let mut out: Vec<PyObjectRef> = Vec::new();
                for f in cur_row {
                    let conv = if dialect.quoting == 2 {
                        if f.is_empty() { py_str(&f) } else { match f.parse::<f64>() { Ok(n) => py_float(n), Err(_) => py_str(&f) } }
                    } else if dialect.quoting == 5 {
                        if f.is_empty() { py_none() } else { py_str(&f) }
                    } else if dialect.quoting == 4 {
                        if f.is_empty() { py_none() } else { match f.parse::<f64>() { Ok(n) => py_float(n), Err(_) => py_str(&f) } }
                    } else { py_str(&f) };
                    out.push(conv);
                }
                rows.push(out);
            }
            if in_quotes && dialect.strict {
                return Err(PyError::Exception("Error".to_string(), PyObjectRef::new(PyObject::Exception{ typ: "Error".to_string(), args: vec![py_str("unexpected end of data")], cause: None, suppress_context: false, context: None, traceback: None, extra: None })));
            }
        }
        Ok(rows)
    }

    fn format_csv_field(field: &str, dialect: &CsvDialect, is_none: bool) -> PyResult<String> {
        let quoting = dialect.quoting;
        let delimiter = dialect.delimiter;
        let quotechar = dialect.quotechar.unwrap_or('"');
        let escapechar = dialect.escapechar;
        let doublequote = dialect.doublequote;
        // Determine if quoting is needed
        let needs_quote = if quoting == 1 {
            true
        } else if quoting == 3 {
            false
        } else if quoting == 2 {
            field.parse::<f64>().is_err()
        } else if quoting == 4 {
            field.parse::<f64>().is_err()
        } else if quoting == 5 {
            !is_none
        } else {
            // QUOTE_MINIMAL: quote if contains delimiter, quotechar (when doublequote), or newline/lineterminator
            let contains_delim = field.contains(delimiter);
            let contains_quote = field.contains(quotechar);
            let contains_nl = field.contains('\n') || field.contains('\r') || field.contains(dialect.lineterminator.as_str());
            if contains_quote && !doublequote {
                // when doublequote false, quote char is escaped, not quoted, unless also contains delim/nl
                contains_delim || contains_nl
            } else {
                contains_delim || contains_quote || contains_nl
            }
        };
        if is_none {
            if quoting == 3 || quoting == 4 || quoting == 5 {
                return Err(PyError::Exception("Error".to_string(), PyObjectRef::new(PyObject::Exception{ typ: "Error".to_string(), args: vec![py_str("need to escape, but no escapechar set")], cause: None, suppress_context: false, context: None, traceback: None, extra: None })));
            }
            return Ok("\"\"".to_string());
        }
        if quoting == 3 {
            let mut esc = String::new();
            for ch in field.chars() {
                if ch == delimiter || ch == quotechar || ch == '\n' || ch == '\r' || Some(ch) == escapechar {
                    if let Some(ec) = escapechar {
                        esc.push(ec); esc.push(ch);
                    } else {
                        return Err(PyError::Exception("Error".to_string(), PyObjectRef::new(PyObject::Exception{ typ: "Error".to_string(), args: vec![py_str("need to escape, but no escapechar set")], cause: None, suppress_context: false, context: None, traceback: None, extra: None })));
                    }
                } else { esc.push(ch); }
            }
            return Ok(esc);
        }
        if needs_quote {
            let mut out = String::new();
            out.push(quotechar);
            for ch in field.chars() {
                if ch == quotechar {
                    if doublequote { out.push(quotechar); out.push(quotechar); }
                    else if let Some(ec) = escapechar { out.push(ec); out.push(ch); }
                    else { return Err(PyError::Exception("Error".to_string(), PyObjectRef::new(PyObject::Exception{ typ: "Error".to_string(), args: vec![py_str("need to escape, but no escapechar set")], cause: None, suppress_context: false, context: None, traceback: None, extra: None }))); }
                } else { out.push(ch); }
            }
            out.push(quotechar);
            Ok(out)
        } else {
            // not quoted, but may need to escape quotechar if doublequote false
            if field.contains(quotechar) && !doublequote {
                if let Some(ec) = escapechar {
                    let mut esc = String::new();
                    for ch in field.chars() {
                        if ch == quotechar { esc.push(ec); esc.push(ch); } else { esc.push(ch); }
                    }
                    return Ok(esc);
                } else {
                    return Err(PyError::Exception("Error".to_string(), PyObjectRef::new(PyObject::Exception{ typ: "Error".to_string(), args: vec![py_str("need to escape, but no escapechar set")], cause: None, suppress_context: false, context: None, traceback: None, extra: None })));
                }
            }
            Ok(field.to_string())
        }
    }

    fn format_csv_row(cells: Vec<PyObjectRef>, dialect: &CsvDialect) -> PyResult<String> {
        // Handle special writer cases matching CPython
        if cells.is_empty() {
            return Ok(dialect.lineterminator.clone());
        }
        // Single field cases
        if cells.len() == 1 {
            let is_none = matches!(&*cells[0].borrow(), PyObject::None);
            let s = if is_none { "".to_string() } else { cells[0].str() };
            if s.is_empty() {
                // [''] or [None] single empty
                if dialect.quoting == 3 {
                    // QUOTE_NONE with single empty should error
                    return Err(PyError::Exception("Error".to_string(), PyObjectRef::new(PyObject::Exception{ typ: "Error".to_string(), args: vec![py_str("need to escape, but no escapechar set")], cause: None, suppress_context: false, context: None, traceback: None, extra: None })));
                }
                // For QUOTE_MINIMAL etc, single empty is '""'
                if is_none && (dialect.quoting == 4 || dialect.quoting == 5) {
                    return Err(PyError::Exception("Error".to_string(), PyObjectRef::new(PyObject::Exception{ typ: "Error".to_string(), args: vec![py_str("need to escape, but no escapechar set")], cause: None, suppress_context: false, context: None, traceback: None, extra: None })));
                }
                return Ok("\"\"".to_string() + &dialect.lineterminator);
            }
            // single non-empty field
            // need to check if field needs special handling for space delimiter etc? Not needed for single
            let formatted = if is_none {
                "\"\"".to_string()
            } else {
                format_csv_field(&s, dialect, false)?
            };
            return Ok(formatted + &dialect.lineterminator);
        }
        // Multiple fields
        let mut parts: Vec<String> = Vec::new();
        for cell in cells.iter() {
            let is_none = matches!(&*cell.borrow(), PyObject::None);
            let s = if is_none { "".to_string() } else { cell.str() };
            if is_none {
                if dialect.quoting == 3 || dialect.quoting == 4 || dialect.quoting == 5 {
                    // For multi-field, QUOTE_NONE with None should be allowed as empty? Check tests: [None,None] with QUOTE_NONE delimiter=' ' skipinitialspace false should be ' ' (empty)
                    // But earlier we error for single None, for multi we allow empty
                    // However test_write_empty_fields_space_delimiter expects Error for [None,None] with delimiter=' ' skipinitialspace True quoting QUOTE_NONE etc
                    // Let's handle generic: for multi, None as empty string, but if delimiter is space and skipinitialspace true -> need quoted empty
                    if dialect.delimiter == ' ' && dialect.skipinitialspace {
                        // need to check if error expected: for QUOTE_NONE etc, [None,None] with space and skipinitialspace True should error
                        if dialect.quoting == 3 || dialect.quoting == 4 || dialect.quoting == 5 {
                            // actually test expects Error for that case, but our current path would produce '"" ""' not error
                            // The test for QUOTE_NONE with space skipinitialspace True expects Error for ['', ''] and [None,None]
                            // So we need to detect that and error
                            if dialect.quoting == 3 {
                                // QUOTE_NONE with space delimiter and skipinitialspace True and empty field -> need escape but no escape? But test has quoting=QUOTE_NONE, delimiter=' ', skipinitialspace=True, ['', ''] -> Error
                                // That's because empty field with space delimiter and skipinitialspace requires quoting but quoting is NONE -> error
                                return Err(PyError::Exception("Error".to_string(), PyObjectRef::new(PyObject::Exception{ typ: "Error".to_string(), args: vec![py_str("need to escape, but no escapechar set")], cause: None, suppress_context: false, context: None, traceback: None, extra: None })));
                            } else {
                                // QUOTE_STRINGS/NOTNULL with space delimiter and None -> error? test says for [None,None] with space delimiter, any quoting (NONE, STRINGS, NOTNULL) with skipinitialspace True should error?
                                // Actually test: for quoting in QUOTE_NONE, STRINGS, NOTNULL: _write_test([None,None], ' ', delimiter=' ', skipinitialspace=False) -> ' ' ; _write_error for skipinitialspace True
                                // So for True, error
                                return Err(PyError::Exception("Error".to_string(), PyObjectRef::new(PyObject::Exception{ typ: "Error".to_string(), args: vec![py_str("need to escape, but no escapechar set")], cause: None, suppress_context: false, context: None, traceback: None, extra: None })));
                            }
                        }
                        parts.push("\"\"".to_string());
                    } else {
                        // For non-space or skip false, None as empty
                        parts.push("".to_string());
                    }
                } else {
                    if dialect.delimiter == ' ' && dialect.skipinitialspace {
                        parts.push("\"\"".to_string());
                    } else {
                        parts.push("".to_string());
                    }
                }
            } else {
                // normal field
                if s.is_empty() {
                    // empty string handling for multi
                    if dialect.quoting == 3 {
                        // QUOTE_NONE with empty string: just empty (no quotes) – but need to consider space delimiter case
                        if dialect.delimiter == ' ' && dialect.skipinitialspace {
                            return Err(PyError::Exception("Error".to_string(), PyObjectRef::new(PyObject::Exception{ typ: "Error".to_string(), args: vec![py_str("need to escape, but no escapechar set")], cause: None, suppress_context: false, context: None, traceback: None, extra: None })));
                        }
                        parts.push("".to_string());
                    } else {
                        // for minimal etc, empty string is '' (empty) not '""' when in multi? Check test: ['', ''] with delimiter ',' skip false -> ',' (two empty fields -> ',')
                        // So empty should be "" not '""' for multi
                        if dialect.delimiter == ' ' && dialect.skipinitialspace {
                            parts.push("\"\"".to_string());
                        } else {
                            parts.push("".to_string());
                        }
                    }
                } else {
                    let f = format_csv_field(&s, dialect, false)?;
                    parts.push(f);
                }
            }
        }
        // Special handling for space delimiter with skipinitialspace True and empty fields already handled
        // For case where some fields were quoted empty etc, join
        Ok(parts.join(&dialect.delimiter.to_string()) + &dialect.lineterminator)
    }

    let mut d = StdHashMap::new();
    macro_rules! csv_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction{ name: $name.to_string(), func: $func }));
        };
    }

    let error_type = PyObjectRef::new(PyObject::Type{ name: "Error".to_string(), dict: Box::new(str_map_to_typedict(StdHashMap::new())), bases: vec![], mro: vec![] });
    d.insert("Error".to_string(), error_type.clone());
    d.insert("QUOTE_MINIMAL".to_string(), py_int(0));
    d.insert("QUOTE_ALL".to_string(), py_int(1));
    d.insert("QUOTE_NONNUMERIC".to_string(), py_int(2));
    d.insert("QUOTE_NONE".to_string(), py_int(3));
    d.insert("QUOTE_STRINGS".to_string(), py_int(4));
    d.insert("QUOTE_NOTNULL".to_string(), py_int(5));

    csv_func!("field_size_limit", |args| {
        if args.is_empty() {
            let lim = CSV_FIELD_LIMIT.with(|c| *c.borrow());
            Ok(py_int(lim as i64))
        } else if args.len()==1 {
            let v = args[0].as_i64().ok_or_else(|| PyError::type_error("field_size_limit() argument must be int"))?;
            if v < 0 { return Err(PyError::type_error("field_size_limit must be non-negative")); }
            CSV_FIELD_LIMIT.with(|c| *c.borrow_mut() = v as usize);
            Ok(py_int(v))
        } else {
            Err(PyError::type_error("field_size_limit() takes at most 1 argument"))
        }
    });

    csv_func!("register_dialect", |args| {
        if args.is_empty() { return Err(PyError::type_error("register_dialect() missing required argument: 'name'")); }
        // check name is string
        let name_obj = &args[0];
        let is_str = matches!(&*name_obj.borrow(), PyObject::Str(_));
        if !is_str {
            return Err(PyError::type_error("dialect name must be a string"));
        }
        let name = name_obj.str();
        // check at most 2 positional args (name, dialect) plus kwargs dict
        if args.len() > 3 {
            return Err(PyError::type_error(format!("register_dialect expected at most 2 arguments, got {}", args.len())));
        }
        if args.len() == 3 {
            let last = &args[2];
            if !matches!(&*last.borrow(), PyObject::Dict(_)) {
                return Err(PyError::type_error(format!("register_dialect expected at most 2 arguments, got {}", args.len())));
            }
        }
        // also check if args.len()==2 and second arg is not dialect nor dict? That's allowed, but if second arg is unexpected type and also kwargs missing, build_dialect will handle
        // but check for extra positional like 0,0 case where args = [name, 0, 0] (len 3, last not dict) already handled
        // also handle case args.len()==2 where second arg is keyword-like? not needed
        // detect bad argument count like register_dialect(None, None) already handled by name check
        // handle case where dialect is passed but kwargs also contains bad argument will be caught by build_dialect
        let (dialect_arg, kwargs) = extract_kwargs(args, 1);
        // if dialect_arg is Some and it's 0 int, and kwargs empty, that's still considered dialect, but test expects TypeError for register_dialect(\"nonesuch\", 0, 0) which we already rejected via len check
        // also need to handle register_dialect(\"nonesuch\", badargument=None) where badargument is kw in dict
        let dialect = build_dialect(dialect_arg, &kwargs)?;
        CSV_DIALECTS.with(|c| c.borrow_mut().insert(name, dialect));
        Ok(py_none())
    });
    csv_func!("unregister_dialect", |args| {
        if args.is_empty() { return Err(PyError::type_error("unregister_dialect() missing required argument")); }
        if args.len() != 1 {
            return Err(PyError::type_error(format!("unregister_dialect expected 1 argument, got {}", args.len())));
        }
        let name_obj = &args[0];
        // if name is not string, still treat as unknown dialect error? Test expects csv.Error for None and \"nonesuch\".
        // But if name is not string like None, we should raise Error with unknown dialect \"None\"
        let name = name_obj.str();
        let is_str = matches!(&*name_obj.borrow(), PyObject::Str(_));
        if !is_str {
            // For None or non-string, CPython raises Error (unknown dialect) not TypeError, except when called with no args or wrong count.
            // So we still go to unknown dialect path.
        }
        let removed = CSV_DIALECTS.with(|c| c.borrow_mut().remove(name.as_str()).is_some());
        if !removed { return Err(PyError::Exception("Error".to_string(), PyObjectRef::new(PyObject::Exception{ typ: "Error".to_string(), args: vec![py_str(&format!("unknown dialect {}", name))], cause: None, suppress_context: false, context: None, traceback: None, extra: None }))); }
        Ok(py_none())
    });
    csv_func!("get_dialect", |args| {
        if args.is_empty() { return Err(PyError::type_error("get_dialect() missing required argument")); }
        if args.len() != 1 {
            return Err(PyError::type_error(format!("get_dialect expected 1 argument, got {}", args.len())));
        }
        let name = args[0].str();
        let dialect = CSV_DIALECTS.with(|c| c.borrow().get(name.as_str()).cloned());
        if let Some(d) = dialect {
            let mut dict = crate::object::AttrMap::new();
            dict.insert_str("delimiter", py_str(&d.delimiter.to_string()));
            dict.insert_str("quotechar", d.quotechar.map(|c| py_str(&c.to_string())).unwrap_or_else(py_none));
            dict.insert_str("escapechar", d.escapechar.map(|c| py_str(&c.to_string())).unwrap_or_else(py_none));
            dict.insert_str("doublequote", py_bool(d.doublequote));
            dict.insert_str("skipinitialspace", py_bool(d.skipinitialspace));
            dict.insert_str("lineterminator", py_str(&d.lineterminator));
            dict.insert_str("quoting", py_int(d.quoting));
            dict.insert_str("strict", py_bool(d.strict));
            let mut type_dict = StdHashMap::new();
            type_dict.insert_str("__copy__", PyObjectRef::new(PyObject::BuiltinFunction{ name: "__copy__".to_string(), func: |_| Err(PyError::type_error("cannot pickle 'Dialect' instances")) }));
            type_dict.insert_str("__deepcopy__", PyObjectRef::new(PyObject::BuiltinFunction{ name: "__deepcopy__".to_string(), func: |_| Err(PyError::type_error("cannot pickle 'Dialect' instances")) }));
            type_dict.insert_str("__reduce__", PyObjectRef::new(PyObject::BuiltinFunction{ name: "__reduce__".to_string(), func: |_| Err(PyError::type_error("cannot pickle 'Dialect' instances")) }));
            type_dict.insert_str("__reduce_ex__", PyObjectRef::new(PyObject::BuiltinFunction{ name: "__reduce_ex__".to_string(), func: |_| Err(PyError::type_error("cannot pickle 'Dialect' instances")) }));
            type_dict.insert_str("__getstate__", PyObjectRef::new(PyObject::BuiltinFunction{ name: "__getstate__".to_string(), func: |_| Err(PyError::type_error("cannot pickle 'Dialect' instances")) }));
            type_dict.insert_str("__setstate__", PyObjectRef::new(PyObject::BuiltinFunction{ name: "__setstate__".to_string(), func: |_| Err(PyError::type_error("cannot pickle 'Dialect' instances")) }));
            type_dict.insert_str("__getnewargs__", PyObjectRef::new(PyObject::BuiltinFunction{ name: "__getnewargs__".to_string(), func: |_| Err(PyError::type_error("cannot pickle 'Dialect' instances")) }));
            type_dict.insert_str("__getnewargs_ex__", PyObjectRef::new(PyObject::BuiltinFunction{ name: "__getnewargs_ex__".to_string(), func: |_| Err(PyError::type_error("cannot pickle 'Dialect' instances")) }));
            type_dict.insert_str("__setattr__", PyObjectRef::new(PyObject::BuiltinFunction{ name: "__setattr__".to_string(), func: |_| Err(PyError::AttributeError("attribute is read-only".to_string())) }));
            type_dict.insert_str("__delattr__", PyObjectRef::new(PyObject::BuiltinFunction{ name: "__delattr__".to_string(), func: |_| Err(PyError::AttributeError("attribute is read-only".to_string())) }));
            let dialect_type = PyObjectRef::new(PyObject::Type{ name: "Dialect".to_string(), dict: Box::new(str_map_to_typedict(type_dict)), bases: vec![], mro: vec![] });
            let dialect_obj = PyObjectRef::new(PyObject::Instance{ typ: dialect_type.clone(), dict });
            // also add copy prevention via __copy__ etc on instance dict? already via type
            Ok(dialect_obj)
        } else {
            Err(PyError::Exception("Error".to_string(), PyObjectRef::new(PyObject::Exception{ typ: "Error".to_string(), args: vec![py_str(&format!("unknown dialect {}", name))], cause: None, suppress_context: false, context: None, traceback: None, extra: None })))
        }
    });
    csv_func!("list_dialects", |args| {
        if !args.is_empty() {
            return Err(PyError::type_error(format!("_csv.list_dialects() takes no arguments ({} given)", args.len())));
        }
        let names: Vec<PyObjectRef> = CSV_DIALECTS.with(|c| c.borrow().keys().map(|k| py_str(k)).collect());
        Ok(py_list(names))
    });

    csv_func!("reader", |args| {
        if args.is_empty() { return Err(PyError::type_error("reader() missing required argument")); }
        let iterable = args[0].clone();
        if matches!(&*iterable.borrow(), PyObject::None) { return Err(PyError::type_error("reader() argument must be iterable")); }
        let (dialect_arg, kwargs) = extract_kwargs(args, 1);
        let dialect = build_dialect(dialect_arg, &kwargs)?;
        let is_file = iterable.borrow().get_attribute("readline").is_ok() || iterable.borrow().get_attribute("_file").and_then(|f| f.borrow().get_attribute("readline")).is_ok();
        let mut rows: Vec<PyObjectRef> = Vec::new();
        let mut is_file_flag = false;
        if is_file {
            is_file_flag = true;
        } else {
            let lines = collect_lines(&iterable)?;
            let parsed_rows = parse_csv_lines(lines, &dialect)?;
            for r in parsed_rows {
                rows.push(py_list(r));
            }
        }
        let reader_type = PyObjectRef::new(PyObject::Type{ name: "reader".to_string(), dict: Box::new(str_map_to_typedict(StdHashMap::new())), bases: vec![], mro: vec![] });
        let mut dict = crate::object::AttrMap::new();
        {
            let mut ddict = crate::object::AttrMap::new();
            ddict.insert_str("delimiter", py_str(&dialect.delimiter.to_string()));
            ddict.insert_str("quotechar", dialect.quotechar.map(|c| py_str(&c.to_string())).unwrap_or_else(py_none));
            ddict.insert_str("escapechar", dialect.escapechar.map(|c| py_str(&c.to_string())).unwrap_or_else(py_none));
            ddict.insert_str("doublequote", py_bool(dialect.doublequote));
            ddict.insert_str("skipinitialspace", py_bool(dialect.skipinitialspace));
            ddict.insert_str("lineterminator", py_str(&dialect.lineterminator));
            ddict.insert_str("quoting", py_int(dialect.quoting));
            ddict.insert_str("strict", py_bool(dialect.strict));
            let mut type_dict = StdHashMap::new();
            type_dict.insert_str("__setattr__", PyObjectRef::new(PyObject::BuiltinFunction{ name: "__setattr__".to_string(), func: |_| Err(PyError::AttributeError("attribute is read-only".to_string())) }));
            type_dict.insert_str("__delattr__", PyObjectRef::new(PyObject::BuiltinFunction{ name: "__delattr__".to_string(), func: |_| Err(PyError::AttributeError("attribute is read-only".to_string())) }));
            let dtype = PyObjectRef::new(PyObject::Type{ name: "Dialect".to_string(), dict: Box::new(str_map_to_typedict(type_dict)), bases: vec![], mro: vec![] });
            let dialect_obj = PyObjectRef::new(PyObject::Instance{ typ: dtype, dict: ddict });
            dict.insert_str("dialect", dialect_obj);
        }
        dict.insert_str("line_num", py_int(0));
        if is_file_flag {
            dict.insert_str("_fileobj", iterable.clone());
            let mut ddict2 = crate::object::AttrMap::new();
            ddict2.insert_str("delimiter", py_str(&dialect.delimiter.to_string()));
            ddict2.insert_str("quotechar", dialect.quotechar.map(|c| py_str(&c.to_string())).unwrap_or_else(py_none));
            ddict2.insert_str("escapechar", dialect.escapechar.map(|c| py_str(&c.to_string())).unwrap_or_else(py_none));
            ddict2.insert_str("doublequote", py_bool(dialect.doublequote));
            ddict2.insert_str("skipinitialspace", py_bool(dialect.skipinitialspace));
            ddict2.insert_str("lineterminator", py_str(&dialect.lineterminator));
            ddict2.insert_str("quoting", py_int(dialect.quoting));
            ddict2.insert_str("strict", py_bool(dialect.strict));
            let mut type_dict2 = StdHashMap::new();
            type_dict2.insert_str("__setattr__", PyObjectRef::new(PyObject::BuiltinFunction{ name: "__setattr__".to_string(), func: |_| Err(PyError::AttributeError("attribute is read-only".to_string())) }));
            type_dict2.insert_str("__delattr__", PyObjectRef::new(PyObject::BuiltinFunction{ name: "__delattr__".to_string(), func: |_| Err(PyError::AttributeError("attribute is read-only".to_string())) }));
            let dtype2 = PyObjectRef::new(PyObject::Type{ name: "Dialect".to_string(), dict: Box::new(str_map_to_typedict(type_dict2)), bases: vec![], mro: vec![] });
            let dialect_obj2 = PyObjectRef::new(PyObject::Instance{ typ: dtype2, dict: ddict2 });
            dict.insert_str("_dialect", dialect_obj2);
            dict.insert_str("_is_file", py_bool(true));
            dict.insert_str("_rows", py_list(vec![]));
            dict.insert_str("_index", py_int(0));
        } else {
            dict.insert_str("_rows", py_list(rows));
            dict.insert_str("_index", py_int(0));
            dict.insert_str("_is_file", py_bool(false));
        }
        let iter_type = PyObjectRef::new(PyObject::Type{ name: "reader".to_string(), dict: Box::new(str_map_to_typedict({
            let mut m = StdHashMap::new();
            m.insert_str("__iter__", PyObjectRef::new(PyObject::BuiltinFunction{ name: "__iter__".to_string(), func: |a| Ok(a[0].clone()) }));
            m.insert_str("__next__", PyObjectRef::new(PyObject::BuiltinFunction{ name: "__next__".to_string(), func: |a| {
                let self_obj = &a[0];
                let is_file = self_obj.borrow().get_attribute("_is_file").map(|v| v.truthy()).unwrap_or(false);
                if is_file {
                    let fileobj = self_obj.borrow().get_attribute("_fileobj").map_err(|_| PyError::runtime_error("no fileobj"))?;
                    let dialect_obj = self_obj.borrow().get_attribute("_dialect").map_err(|_| PyError::runtime_error("no dialect"))?;
                    let dialect = {
                        let db = dialect_obj.borrow();
                        if let PyObject::Instance{ dict, .. } = &*db {
                            let delim = dict.get("delimiter").map(|v| v.str().chars().next().unwrap_or(',')).unwrap_or(',');
                            let qc = dict.get("quotechar").and_then(|v| if matches!(&*v.borrow(), PyObject::None) { None } else { Some(v.str().chars().next().unwrap()) });
                            let ec = dict.get("escapechar").and_then(|v| if matches!(&*v.borrow(), PyObject::None) { None } else { Some(v.str().chars().next().unwrap()) });
                            let dq = dict.get("doublequote").map(|v| v.truthy()).unwrap_or(true);
                            let sis = dict.get("skipinitialspace").map(|v| v.truthy()).unwrap_or(false);
                            let lt = dict.get("lineterminator").map(|v| v.str()).unwrap_or_else(|| "\r\n".to_string());
                            let q = dict.get("quoting").and_then(|v| v.as_i64()).unwrap_or(0);
                            let strict = dict.get("strict").map(|v| v.truthy()).unwrap_or(false);
                            CsvDialect{ delimiter: delim, quotechar: qc, escapechar: ec, doublequote: dq, skipinitialspace: sis, lineterminator: lt, quoting: q, strict }
                        } else { CsvDialect::excel() }
                    };
                    let quotechar = if dialect.quoting == 3 { None } else { dialect.quotechar };
                    let escapechar = dialect.escapechar;
                    let delimiter = dialect.delimiter;
                    let mut row: Vec<String> = Vec::new();
                    let mut field = String::new();
                    let mut in_quotes = false;
                    // need to handle that we may have leftover? For simplicity, read one row per __next__ call, handling multiline via loop
                    loop {
                        let actual_file = fileobj.borrow().get_attribute("_file").unwrap_or_else(|_| fileobj.clone());
                        let line_res = crate::object::with_vm_mut(|vm| crate::object::call_method_rebound(vm, &actual_file, "readline", vec![]));
                        let line_obj = match line_res {
                            Ok(Ok(v)) => v,
                            Ok(Err(e)) => return Err(e),
                            Err(e) => return Err(e),
                        };
                        let mut s = line_obj.str();
                        // readline returns "" on EOF
                        if s.is_empty() {
                            if row.is_empty() && field.is_empty() && !in_quotes {
                                return Err(PyError::StopIteration);
                            }
                            // EOF with pending data: treat as end of row
                            if in_quotes && dialect.strict {
                                return Err(PyError::Exception("Error".to_string(), PyObjectRef::new(PyObject::Exception{ typ: "Error".to_string(), args: vec![py_str("unexpected end of data")], cause: None, suppress_context: false, context: None, traceback: None, extra: None })));
                            }
                            // push last field
                            row.push(field.clone());
                            // check field size limit and quoting conversion
                            let limit = CSV_FIELD_LIMIT.with(|c| *c.borrow());
                            for f in &row { if f.len() > limit { return Err(PyError::Exception("Error".to_string(), PyObjectRef::new(PyObject::Exception{ typ: "Error".to_string(), args: vec![py_str("field larger than field limit")], cause: None, suppress_context: false, context: None, traceback: None, extra: None }))); } }
                            let mut out: Vec<PyObjectRef> = Vec::new();
                            for f in row {
                                let conv = if dialect.quoting == 2 {
                                    if f.is_empty() { py_str(&f) } else { match f.parse::<f64>() { Ok(n) => py_float(n), Err(_) => py_str(&f) } }
                                } else if dialect.quoting == 5 {
                                    if f.is_empty() { py_none() } else { py_str(&f) }
                                } else if dialect.quoting == 4 {
                                    if f.is_empty() { py_none() } else { match f.parse::<f64>() { Ok(n) => py_float(n), Err(_) => py_str(&f) } }
                                } else { py_str(&f) };
                                out.push(conv);
                            }
                            if out.is_empty() {
                                // empty line => [[]]? But readline loop for file would not have this? For file with blank line, readline returns "\n", not ""
                                // So this case is EOF with pending, not blank line
                            }
                            let prev = self_obj.borrow().get_attribute("line_num").ok().and_then(|v| v.as_i64()).unwrap_or(0);
                            self_obj.borrow_mut().set_attribute("line_num", py_int(prev+1)).ok();
                            return Ok(py_list(out));
                        }
                        // Remove trailing \r\n or \n or \r for processing, but keep as terminator handling
                        let had_newline = s.ends_with('\n') || s.ends_with('\r');
                        let mut line_content = s.clone();
                        if line_content.ends_with("\r\n") { line_content.truncate(line_content.len()-2); }
                        else if line_content.ends_with('\n') || line_content.ends_with('\r') { line_content.pop(); }
                        else { line_content = s.clone(); } // no newline at EOF
                        // If line is empty and not in quotes, it's a blank line => return [[]]
                        if line_content.is_empty() && !in_quotes && row.is_empty() && field.is_empty() {
                            let prev = self_obj.borrow().get_attribute("line_num").ok().and_then(|v| v.as_i64()).unwrap_or(0);
                            self_obj.borrow_mut().set_attribute("line_num", py_int(prev+1)).ok();
                            return Ok(py_list(vec![]));
                        }
                        let mut chars = line_content.chars().peekable();
                        if !in_quotes && field.is_empty() && row.is_empty() && dialect.skipinitialspace {
                            while chars.peek() == Some(&' ') { chars.next(); }
                            if chars.peek().is_none() {
                                // line was all spaces
                                let prev = self_obj.borrow().get_attribute("line_num").ok().and_then(|v| v.as_i64()).unwrap_or(0);
                                self_obj.borrow_mut().set_attribute("line_num", py_int(prev+1)).ok();
                                return Ok(py_list(vec![py_str("")]));
                            }
                        }
                        while let Some(c) = chars.next() {
                            if in_quotes {
                                if Some(c) == escapechar {
                                    if let Some(nxt) = chars.next() { field.push(nxt); } else {
                                        if dialect.strict { return Err(PyError::Exception("Error".to_string(), PyObjectRef::new(PyObject::Exception{ typ: "Error".to_string(), args: vec![py_str("unexpected end of data")], cause: None, suppress_context: false, context: None, traceback: None, extra: None }))); } else { field.push('\n'); }
                                    }
                                } else if Some(c) == quotechar {
                                    if dialect.doublequote && chars.peek() == Some(&c) { chars.next(); field.push(c); } else {
                                        in_quotes = false;
                                        if let Some(&next) = chars.peek() {
                                            if next != delimiter { if dialect.strict { return Err(PyError::Exception("Error".to_string(), PyObjectRef::new(PyObject::Exception{ typ: "Error".to_string(), args: vec![py_str(&format!("'{}' expected after '\"'", delimiter))], cause: None, suppress_context: false, context: None, traceback: None, extra: None }))); } }
                                        }
                                    }
                                } else { field.push(c); }
                            } else {
                                if Some(c) == escapechar {
                                    if let Some(nxt) = chars.next() { field.push(nxt); } else {
                                        if dialect.strict { return Err(PyError::Exception("Error".to_string(), PyObjectRef::new(PyObject::Exception{ typ: "Error".to_string(), args: vec![py_str("unexpected end of data")], cause: None, suppress_context: false, context: None, traceback: None, extra: None }))); } else { field.push('\n'); }
                                    }
                                } else if Some(c) == quotechar {
                                    if field.is_empty() { in_quotes = true; } else {
                                        if dialect.strict { return Err(PyError::Exception("Error".to_string(), PyObjectRef::new(PyObject::Exception{ typ: "Error".to_string(), args: vec![py_str("unexpected quote")], cause: None, suppress_context: false, context: None, traceback: None, extra: None }))); } else { field.push(c); }
                                    }
                                } else if c == delimiter {
                                    row.push(field.clone()); field = String::new();
                                    if dialect.skipinitialspace { while chars.peek() == Some(&' ') { chars.next(); } }
                                } else { field.push(c); }
                            }
                        }
                        if in_quotes {
                            field.push('\n');
                            continue;
                        } else {
                            // end of line, not in quotes => row complete
                            row.push(field.clone());
                            let limit = CSV_FIELD_LIMIT.with(|c| *c.borrow());
                            for f in &row { if f.len() > limit { return Err(PyError::Exception("Error".to_string(), PyObjectRef::new(PyObject::Exception{ typ: "Error".to_string(), args: vec![py_str("field larger than field limit")], cause: None, suppress_context: false, context: None, traceback: None, extra: None }))); } }
                            let mut out: Vec<PyObjectRef> = Vec::new();
                            for f in row {
                                let conv = if dialect.quoting == 2 {
                                    if f.is_empty() { py_str(&f) } else { match f.parse::<f64>() { Ok(n) => py_float(n), Err(_) => py_str(&f) } }
                                } else if dialect.quoting == 5 {
                                    if f.is_empty() { py_none() } else { py_str(&f) }
                                } else if dialect.quoting == 4 {
                                    if f.is_empty() { py_none() } else { match f.parse::<f64>() { Ok(n) => py_float(n), Err(_) => py_str(&f) } }
                                } else { py_str(&f) };
                                out.push(conv);
                            }
                            let prev = self_obj.borrow().get_attribute("line_num").ok().and_then(|v| v.as_i64()).unwrap_or(0);
                            self_obj.borrow_mut().set_attribute("line_num", py_int(prev+1)).ok();
                            return Ok(py_list(out));
                        }
                    }
                } else {
                    let rows = self_obj.borrow().get_attribute("_rows").map_err(|_| PyError::runtime_error("no _rows"))?;
                    let idx_obj = self_obj.borrow().get_attribute("_index").map_err(|_| PyError::runtime_error("no _index"))?;
                    let idx = idx_obj.as_i64().unwrap_or(0) as usize;
                    let rows_b = rows.borrow();
                    if let PyObject::List(items) = &*rows_b {
                        if idx >= items.len() { return Err(PyError::StopIteration); }
                        let row = items[idx].clone();
                        drop(rows_b);
                        self_obj.borrow_mut().set_attribute("_index", py_int((idx+1) as i64)).ok();
                        self_obj.borrow_mut().set_attribute("line_num", py_int((idx+1) as i64)).ok();
                        Ok(row)
                    } else { Err(PyError::StopIteration) }
                }
            }}));
            m
        })), bases: vec![], mro: vec![] });
        let reader_obj = PyObjectRef::new(PyObject::Instance{ typ: reader_type.clone(), dict });
        if let PyObject::Instance{ typ, .. } = &mut *reader_obj.borrow_mut() { *typ = iter_type; }
        Ok(reader_obj)
    });

    csv_func!("writer", |args| {
        if args.is_empty() { return Err(PyError::type_error("writer() missing required argument")); }
        let fileobj = args[0].clone();
        // Check if write is a property whose getter raises (e.g. BadWriter)
        if let PyObject::Instance { typ, .. } = &*fileobj.borrow() {
            if let Some(prop) = crate::object::lookup_dunder_via_mro(typ, "write") {
                if prop.borrow().type_name() == "property" {
                    if let Ok(fget) = prop.borrow().get_attribute("fget") {
                        if !matches!(&*fget.borrow(), PyObject::None) {
                            let res = crate::object::with_vm_mut(|vm| crate::object::call_function_disposable(&fget, vec![fileobj.clone()], vec![]));
                            match res {
                                Ok(Err(e)) => return Err(e),
                                Err(e) => return Err(e),
                                _ => {}
                            }
                        }
                    }
                }
            }
        }
        if matches!(&*fileobj.borrow(), PyObject::None) {
            return Err(PyError::type_error("argument 1 must have a \"write\" method"));
        }
        let has_write = fileobj.borrow().get_attribute("write").is_ok();
        if !has_write {
            let has_getattr = {
                let b = fileobj.borrow();
                if let PyObject::Instance { typ, .. } = &*b {
                    crate::object::lookup_dunder_via_mro(typ, "__getattr__").is_some()
                } else { false }
            };
            if !has_getattr {
                return Err(PyError::type_error("argument 1 must have a \"write\" method"));
            }
        }
        let (dialect_arg, kwargs) = extract_kwargs(args, 1);
        let dialect = build_dialect(dialect_arg, &kwargs)?;
        let writer_type = PyObjectRef::new(PyObject::Type{ name: "writer".to_string(), dict: Box::new(str_map_to_typedict(StdHashMap::new())), bases: vec![], mro: vec![] });
        let mut dict = crate::object::AttrMap::new();
        {
            let mut ddict = crate::object::AttrMap::new();
            ddict.insert_str("delimiter", py_str(&dialect.delimiter.to_string()));
            ddict.insert_str("quotechar", dialect.quotechar.map(|c| py_str(&c.to_string())).unwrap_or_else(py_none));
            ddict.insert_str("escapechar", dialect.escapechar.map(|c| py_str(&c.to_string())).unwrap_or_else(py_none));
            ddict.insert_str("doublequote", py_bool(dialect.doublequote));
            ddict.insert_str("skipinitialspace", py_bool(dialect.skipinitialspace));
            ddict.insert_str("lineterminator", py_str(&dialect.lineterminator));
            ddict.insert_str("quoting", py_int(dialect.quoting));
            ddict.insert_str("strict", py_bool(dialect.strict));
            let mut type_dict = StdHashMap::new();
            type_dict.insert_str("__setattr__", PyObjectRef::new(PyObject::BuiltinFunction{ name: "__setattr__".to_string(), func: |_| Err(PyError::AttributeError("attribute is read-only".to_string())) }));
            type_dict.insert_str("__delattr__", PyObjectRef::new(PyObject::BuiltinFunction{ name: "__delattr__".to_string(), func: |_| Err(PyError::AttributeError("attribute is read-only".to_string())) }));
            let dtype = PyObjectRef::new(PyObject::Type{ name: "Dialect".to_string(), dict: Box::new(str_map_to_typedict(type_dict)), bases: vec![], mro: vec![] });
            let dialect_obj = PyObjectRef::new(PyObject::Instance{ typ: dtype, dict: ddict });
            dict.insert_str("dialect", dialect_obj);
        }
        dict.insert_str("_fileobj", fileobj.clone());
        dict.insert_str("_dialect", {
            let mut ddict = crate::object::AttrMap::new();
            ddict.insert_str("delimiter", py_str(&dialect.delimiter.to_string()));
            ddict.insert_str("quotechar", dialect.quotechar.map(|c| py_str(&c.to_string())).unwrap_or_else(py_none));
            ddict.insert_str("escapechar", dialect.escapechar.map(|c| py_str(&c.to_string())).unwrap_or_else(py_none));
            ddict.insert_str("doublequote", py_bool(dialect.doublequote));
            ddict.insert_str("skipinitialspace", py_bool(dialect.skipinitialspace));
            ddict.insert_str("lineterminator", py_str(&dialect.lineterminator));
            ddict.insert_str("quoting", py_int(dialect.quoting));
            ddict.insert_str("strict", py_bool(dialect.strict));
            let mut type_dict = StdHashMap::new();
            type_dict.insert_str("__setattr__", PyObjectRef::new(PyObject::BuiltinFunction{ name: "__setattr__".to_string(), func: |_| Err(PyError::AttributeError("attribute is read-only".to_string())) }));
            type_dict.insert_str("__delattr__", PyObjectRef::new(PyObject::BuiltinFunction{ name: "__delattr__".to_string(), func: |_| Err(PyError::AttributeError("attribute is read-only".to_string())) }));
            let dtype = PyObjectRef::new(PyObject::Type{ name: "Dialect".to_string(), dict: Box::new(str_map_to_typedict(type_dict)), bases: vec![], mro: vec![] });
            PyObjectRef::new(PyObject::Instance{ typ: dtype, dict: ddict })
        });
        let writer_obj = PyObjectRef::new(PyObject::Instance{ typ: writer_type.clone(), dict });
        if let PyObject::Type{ dict: tdict, .. } = &mut *writer_type.borrow_mut() {
            tdict.insert_str("writerow", PyObjectRef::new(PyObject::BuiltinFunction{ name: "writerow".to_string(), func: |a| {
                if a.len()<2 { return Err(PyError::type_error("writerow() missing argument")); }
                let self_obj = &a[0];
                let row_arg = &a[1];
                let fobj = self_obj.borrow().get_attribute("_fileobj").map_err(|_| PyError::runtime_error("no fileobj"))?;
                let dialect_obj = self_obj.borrow().get_attribute("_dialect").map_err(|_| PyError::runtime_error("no dialect"))?;
                let dialect = {
                    let db = dialect_obj.borrow();
                    if let PyObject::Instance{ dict, .. } = &*db {
                        let delim = dict.get("delimiter").map(|v| v.str().chars().next().unwrap_or(',')).unwrap_or(',');
                        let qc = dict.get("quotechar").and_then(|v| if matches!(&*v.borrow(), PyObject::None) { None } else { Some(v.str().chars().next().unwrap()) });
                        let ec = dict.get("escapechar").and_then(|v| if matches!(&*v.borrow(), PyObject::None) { None } else { Some(v.str().chars().next().unwrap()) });
                        let dq = dict.get("doublequote").map(|v| v.truthy()).unwrap_or(true);
                        let sis = dict.get("skipinitialspace").map(|v| v.truthy()).unwrap_or(false);
                        let lt = dict.get("lineterminator").map(|v| v.str()).unwrap_or_else(|| "\r\n".to_string());
                        let q = dict.get("quoting").and_then(|v| v.as_i64()).unwrap_or(0);
                        let strict = dict.get("strict").map(|v| v.truthy()).unwrap_or(false);
                        CsvDialect{ delimiter: delim, quotechar: qc, escapechar: ec, doublequote: dq, skipinitialspace: sis, lineterminator: lt, quoting: q, strict }
                    } else { CsvDialect::excel() }
                };
                let iter = crate::object::builtin_iter(&[row_arg.clone()])?;
                let mut cells: Vec<PyObjectRef> = Vec::new();
                loop {
                    match crate::object::builtin_next(&[iter.clone()]) {
                        Ok(v) => cells.push(v),
                        Err(PyError::StopIteration) => break,
                        Err(e) => return Err(e),
                    }
                }
                let out = format_csv_row(cells, &dialect)?;
                let actual_fobj = fobj.borrow().get_attribute("_file").unwrap_or_else(|_| fobj.clone());
                let write_res = crate::object::with_vm_mut(|vm| crate::object::call_method_rebound(vm, &actual_fobj, "write", vec![py_str(&out)]));
                match write_res {
                    Ok(Ok(_)) => {},
                    Ok(Err(e)) => return Err(e),
                    Err(e) => return Err(e),
                }
                Ok(py_int(out.len() as i64))
            }}));
            tdict.insert_str("writerows", PyObjectRef::new(PyObject::BuiltinFunction{ name: "writerows".to_string(), func: |a| {
                if a.len()<2 { return Err(PyError::type_error("writerows() missing argument")); }
                let self_obj = &a[0];
                let rows_arg = &a[1];
                let iter = crate::object::builtin_iter(&[rows_arg.clone()])?;
                let wrow = self_obj.borrow().get_attribute("writerow").unwrap();
                loop {
                    match crate::object::builtin_next(&[iter.clone()]) {
                        Ok(row) => { crate::object::call_function_disposable(&wrow, vec![self_obj.clone(), row.clone()], vec![])?; },
                        Err(PyError::StopIteration) => break,
                        Err(e) => return Err(e),
                    }
                }
                Ok(py_none())
            }}));
        }
        // writerow/writerows via type
        if let PyObject::Instance{ typ, .. } = &mut *writer_obj.borrow_mut() { *typ = writer_type.clone(); }
        Ok(writer_obj)
    });

    fn validate_dialect_obj(obj: &PyObjectRef) -> PyResult<()> {
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
    d.insert("__version__".to_string(), py_str("1.0"));
    d.insert("__doc__".to_string(), py_str("CSV parsing and writing"));

    let dict_reader_type = PyObjectRef::new(PyObject::Type{ name: "DictReader".to_string(), dict: Box::new(str_map_to_typedict(StdHashMap::new())), bases: vec![], mro: vec![] });
    if let PyObject::Type{ dict, .. } = &mut *dict_reader_type.borrow_mut() {
        dict.insert_str("__init__", PyObjectRef::new(PyObject::BuiltinFunction{ name: "__init__".to_string(), func: |args| {
            if args.len()<2 { return Err(PyError::type_error("DictReader() missing required argument: 'csvfile'")); }
            let self_obj = &args[0];
            let csvfile = &args[1];
            let mut fieldnames_opt: Option<Vec<PyObjectRef>> = None;
            let mut restkey: Option<PyObjectRef> = None;
            let mut restval: Option<PyObjectRef> = None;
            let mut dialect_arg: Option<PyObjectRef> = None;
            let mut kwargs: StdHashMap<String, PyObjectRef> = StdHashMap::new();
            let mut idx = 2;
            if args.len()>idx {
                let v = &args[idx];
                let is_kwargs = if let PyObject::Dict(d) = &*v.borrow() {
                    d.items().iter().any(|(k,_)| matches!(k.str().as_str(), "restkey"|"restval"|"fieldnames"|"dialect"|"delimiter"|"quotechar"|"escapechar"|"doublequote"|"skipinitialspace"|"lineterminator"|"quoting"|"strict"))
                } else { false };
                if !is_kwargs {
                    if matches!(&*v.borrow(), PyObject::None) {
                        fieldnames_opt = None;
                    } else {
                        let s = v.str();
                        let is_dialect_name = CSV_DIALECTS.with(|c| c.borrow().contains_key(s.as_str()));
                        if is_dialect_name {
                            dialect_arg = Some(v.clone());
                        } else {
                            if let PyObject::List(items) = &*v.borrow() { fieldnames_opt = Some(items.clone()); }
                            else if let PyObject::Tuple(items) = &*v.borrow() { fieldnames_opt = Some(items.clone()); }
                            else if let PyObject::ListIter { list, index } = &*v.borrow() { fieldnames_opt = Some(list[*index..].to_vec()); }
                            else {
                                match crate::object::builtin_iter(&[v.clone()]) {
                                    Ok(it) => {
                                        let mut items = Vec::new();
                                        loop {
                                            match crate::object::builtin_next(&[it.clone()]) {
                                                Ok(val) => items.push(val),
                                                Err(PyError::StopIteration) => break,
                                                Err(e) => return Err(e),
                                            }
                                        }
                                        fieldnames_opt = Some(items);
                                    },
                                    Err(_) => { fieldnames_opt = None; }
                                }
                            }
                        }
                    }
                    idx+=1;
                }
            }
            if args.len()>idx {
                let v = &args[idx];
                if let PyObject::Dict(d) = &*v.borrow() {
                    for (k,val) in d.items() {
                        let ks = k.str();
                        match ks.as_str() {
                            "fieldnames" => {
                                if matches!(&*val.borrow(), PyObject::None) { fieldnames_opt = None; }
                                else if let PyObject::List(items) = &*val.borrow() { fieldnames_opt = Some(items.clone()); }
                                else if let PyObject::ListIter { list, index } = &*val.borrow() { fieldnames_opt = Some(list[*index..].to_vec()); }
                                else {
                                    match crate::object::builtin_iter(&[val.clone()]) {
                                        Ok(it) => {
                                            let mut items = Vec::new();
                                            loop {
                                                match crate::object::builtin_next(&[it.clone()]) { Ok(x)=> items.push(x), Err(PyError::StopIteration)=> break, Err(e)=> return Err(e), }
                                            }
                                            fieldnames_opt = Some(items);
                                        },
                                        Err(_) => fieldnames_opt = None,
                                    }
                                }
                            },
                            "restkey" => restkey = Some(val.clone()),
                            "restval" => restval = Some(val.clone()),
                            "dialect" => dialect_arg = Some(val.clone()),
                            _ => { kwargs.insert(ks, val.clone()); }
                        }
                    }
                    idx+=1;
                } else {
                    dialect_arg = Some(v.clone());
                    idx+=1;
                    if args.len()>idx {
                        if let PyObject::Dict(d) = &*args[idx].borrow() {
                            for (k,val) in d.items() {
                                let ks = k.str();
                                match ks.as_str() {
                                    "restkey" => restkey = Some(val.clone()),
                                    "restval" => restval = Some(val.clone()),
                                    _ => { kwargs.insert(ks, val.clone()); }
                                }
                            }
                        }
                    }
                }
            }
            if args.len()>idx {
                if let PyObject::Dict(d) = &*args.last().unwrap().borrow() {
                    for (k,val) in d.items() {
                        let ks = k.str();
                        if !kwargs.contains_key(&ks) && ks!="fieldnames" && ks!="restkey" && ks!="restval" && ks!="dialect" {
                            kwargs.insert(ks, val.clone());
                        }
                    }
                }
            }
            let dialect = build_dialect(dialect_arg, &kwargs)?;
            let lines = collect_lines(csvfile)?;
            let rows = parse_csv_lines(lines, &dialect)?;
            let mut fieldnames_vec: Vec<PyObjectRef> = Vec::new();
            let mut data_start = 0;
            let mut fieldnames_is_none = false;
            if let Some(fn_vec) = fieldnames_opt {
                fieldnames_vec = fn_vec;
            } else {
                if !rows.is_empty() {
                    fieldnames_vec = rows[0].iter().map(|c| py_str(&c.str())).collect();
                    data_start = 1;
                } else {
                    fieldnames_is_none = true;
                }
            }
            let data_rows = if data_start < rows.len() { rows[data_start..].to_vec() } else { Vec::new() };
            if fieldnames_is_none && fieldnames_vec.is_empty() {
                self_obj.borrow_mut().set_attribute("fieldnames", py_none()).ok();
            } else {
                self_obj.borrow_mut().set_attribute("fieldnames", py_list(fieldnames_vec.clone())).ok();
            }
            self_obj.borrow_mut().set_attribute("restkey", restkey.unwrap_or_else(py_none)).ok();
            self_obj.borrow_mut().set_attribute("restval", restval.unwrap_or_else(py_none)).ok();
            self_obj.borrow_mut().set_attribute("dialect", {
                let mut ddict = crate::object::AttrMap::new();
                ddict.insert_str("delimiter", py_str(&dialect.delimiter.to_string()));
                ddict.insert_str("quotechar", dialect.quotechar.map(|c| py_str(&c.to_string())).unwrap_or_else(py_none));
                ddict.insert_str("escapechar", dialect.escapechar.map(|c| py_str(&c.to_string())).unwrap_or_else(py_none));
                ddict.insert_str("doublequote", py_bool(dialect.doublequote));
                ddict.insert_str("skipinitialspace", py_bool(dialect.skipinitialspace));
                ddict.insert_str("lineterminator", py_str(&dialect.lineterminator));
                ddict.insert_str("quoting", py_int(dialect.quoting));
                ddict.insert_str("strict", py_bool(dialect.strict));
                let mut type_dict = StdHashMap::new();
            type_dict.insert_str("__setattr__", PyObjectRef::new(PyObject::BuiltinFunction{ name: "__setattr__".to_string(), func: |_| Err(PyError::AttributeError("attribute is read-only".to_string())) }));
            type_dict.insert_str("__delattr__", PyObjectRef::new(PyObject::BuiltinFunction{ name: "__delattr__".to_string(), func: |_| Err(PyError::AttributeError("attribute is read-only".to_string())) }));
            let dtype = PyObjectRef::new(PyObject::Type{ name: "Dialect".to_string(), dict: Box::new(str_map_to_typedict(type_dict)), bases: vec![], mro: vec![] });
                PyObjectRef::new(PyObject::Instance{ typ: dtype, dict: ddict })
            }).ok();
            self_obj.borrow_mut().set_attribute("line_num", py_int(0)).ok();
            let mut dict_rows = Vec::new();
            for row in data_rows {
                if row.is_empty() { continue; }
                let d = crate::object::py_dict();
                {
                    let mut db = d.borrow_mut();
                    if let PyObject::Dict(pd) = &mut *db {
                        for (i, fname) in fieldnames_vec.iter().enumerate() {
                            let key = fname.str();
                            let val = row.get(i).cloned().unwrap_or_else(|| {
                                self_obj.borrow().get_attribute("restval").ok().filter(|v| !matches!(&*v.borrow(), PyObject::None)).unwrap_or_else(py_none)
                            });
                            pd.set(py_str(&key), val).ok();
                        }
                        if row.len() > fieldnames_vec.len() {
                            let rk = self_obj.borrow().get_attribute("restkey").unwrap_or_else(|_| py_none());
                            let extra: Vec<PyObjectRef> = row[fieldnames_vec.len()..].to_vec();
                            if matches!(&*rk.borrow(), PyObject::None) {
                                pd.set(py_none(), py_list(extra)).ok();
                            } else {
                                pd.set(rk.clone(), py_list(extra)).ok();
                            }
                        }
                    }
                }
                dict_rows.push(d);
            }
            self_obj.borrow_mut().set_attribute("_dict_rows", py_list(dict_rows)).ok();
            self_obj.borrow_mut().set_attribute("_dict_index", py_int(0)).ok();
            Ok(py_none())
        }}));
        dict.insert_str("__iter__", PyObjectRef::new(PyObject::BuiltinFunction{ name: "__iter__".to_string(), func: |a| Ok(a[0].clone()) }));
        dict.insert_str("__next__", PyObjectRef::new(PyObject::BuiltinFunction{ name: "__next__".to_string(), func: |a| {
            let self_obj = &a[0];
            let idx_obj = self_obj.borrow().get_attribute("_dict_index").map_err(|_| PyError::runtime_error("no _dict_index"))?;
            let idx = idx_obj.as_i64().unwrap_or(0) as usize;
            let rows = self_obj.borrow().get_attribute("_dict_rows").map_err(|_| PyError::runtime_error("no _dict_rows"))?;
            let rb = rows.borrow();
            if let PyObject::List(items) = &*rb {
                if idx >= items.len() { return Err(PyError::StopIteration); }
                let row = items[idx].clone();
                drop(rb);
                self_obj.borrow_mut().set_attribute("_dict_index", py_int((idx+1) as i64)).ok();
                let prev = self_obj.borrow().get_attribute("line_num").ok().and_then(|v| v.as_i64()).unwrap_or(0);
                self_obj.borrow_mut().set_attribute("line_num", py_int(prev+1)).ok();
                Ok(row)
            } else { Err(PyError::StopIteration) }
        }}));
    }
    d.insert("DictReader".to_string(), dict_reader_type.clone());

    let dict_writer_type = PyObjectRef::new(PyObject::Type{ name: "DictWriter".to_string(), dict: Box::new(str_map_to_typedict(StdHashMap::new())), bases: vec![], mro: vec![] });
    if let PyObject::Type{ dict, .. } = &mut *dict_writer_type.borrow_mut() {
        dict.insert_str("__init__", PyObjectRef::new(PyObject::BuiltinFunction{ name: "__init__".to_string(), func: |args| {
            if args.len()<3 { return Err(PyError::type_error("DictWriter() missing required argument: 'fieldnames'")); }
            let self_obj = &args[0];
            let fileobj = args[1].clone();
            let mut restval: Option<PyObjectRef> = None;
            let mut extrasaction = "raise".to_string();
            let mut dialect_arg: Option<PyObjectRef> = None;
            let mut kwargs: StdHashMap<String, PyObjectRef> = StdHashMap::new();
            let fieldnames_arg_raw = &args[2];
            let fieldnames_arg = if let PyObject::Dict(d) = &*fieldnames_arg_raw.borrow() {
                let mut found = None;
                for (k, v) in d.items() {
                    if k.str() == "fieldnames" {
                        found = Some(v.clone());
                        break;
                    }
                }
                if let Some(v) = found {
                    for (k2, v2) in d.items() {
                        let ks2 = k2.str();
                        if ks2 == "fieldnames" { continue; }
                        match ks2.as_str() {
                            "restval" => restval = Some(v2.clone()),
                            "extrasaction" => extrasaction = v2.str().to_lowercase(),
                            "dialect" => dialect_arg = Some(v2.clone()),
                            _ => { kwargs.insert(ks2.clone(), v2.clone()); }
                        }
                    }
                    v
                } else {
                    fieldnames_arg_raw.clone()
                }
            } else {
                fieldnames_arg_raw.clone()
            };
            let mut fieldnames: Vec<PyObjectRef> = Vec::new();
            if let PyObject::List(items) = &*fieldnames_arg.borrow() { fieldnames = items.clone(); }
            else if let PyObject::Tuple(items) = &*fieldnames_arg.borrow() { fieldnames = items.clone(); }
            else if let PyObject::ListIter { list, index } = &*fieldnames_arg.borrow() { fieldnames = list[*index..].to_vec(); }
            else {
                match crate::object::builtin_iter(&[fieldnames_arg.clone()]) {
                    Ok(it) => {
                        loop {
                            match crate::object::builtin_next(&[it.clone()]) { Ok(v)=> fieldnames.push(v), Err(PyError::StopIteration)=> break, Err(e)=> return Err(e), }
                        }
                    },
                    Err(_) => return Err(PyError::type_error("fieldnames must be iterable")),
                }
            }
            let mut idx=3;
            if args.len()>idx {
                let v = &args[idx];
                if let PyObject::Dict(d) = &*v.borrow() {
                    for (k,val) in d.items() {
                        let ks = k.str();
                        match ks.as_str() {
                            "restval" => restval = Some(val.clone()),
                            "extrasaction" => extrasaction = val.str().to_lowercase(),
                            "dialect" => dialect_arg = Some(val.clone()),
                            _ => { kwargs.insert(ks, val.clone()); }
                        }
                    }
                    idx+=1;
                } else {
                    dialect_arg = Some(v.clone());
                    idx+=1;
                    if args.len()>idx {
                        if let PyObject::Dict(d) = &*args[idx].borrow() {
                            for (k,val) in d.items() {
                                let ks = k.str();
                                match ks.as_str() {
                                    "restval" => restval = Some(val.clone()),
                                    "extrasaction" => extrasaction = val.str().to_lowercase(),
                                    _ => { kwargs.insert(ks, val.clone()); }
                                }
                            }
                        }
                    }
                }
            }
            if args.len()>idx {
                if let PyObject::Dict(d) = &*args.last().unwrap().borrow() {
                    for (k,val) in d.items() {
                        let ks = k.str();
                        if !kwargs.contains_key(&ks) && ks!="restval" && ks!="extrasaction" && ks!="dialect" {
                            kwargs.insert(ks, val.clone());
                        }
                    }
                }
            }
            if extrasaction!="raise" && extrasaction!="ignore" {
                return Err(PyError::value_error(format!("extrasaction ({}) must be 'raise' or 'ignore'", extrasaction)));
            }
            let dialect = build_dialect(dialect_arg, &kwargs)?;
            self_obj.borrow_mut().set_attribute("fieldnames", py_list(fieldnames.clone())).ok();
            self_obj.borrow_mut().set_attribute("restval", restval.unwrap_or_else(|| py_str(""))).ok();
            self_obj.borrow_mut().set_attribute("extrasaction", py_str(&extrasaction)).ok();
            self_obj.borrow_mut().set_attribute("_fileobj", fileobj.clone()).ok();
            self_obj.borrow_mut().set_attribute("_dialect", {
                let mut ddict = crate::object::AttrMap::new();
                ddict.insert_str("delimiter", py_str(&dialect.delimiter.to_string()));
                ddict.insert_str("quotechar", dialect.quotechar.map(|c| py_str(&c.to_string())).unwrap_or_else(py_none));
                ddict.insert_str("escapechar", dialect.escapechar.map(|c| py_str(&c.to_string())).unwrap_or_else(py_none));
                ddict.insert_str("doublequote", py_bool(dialect.doublequote));
                ddict.insert_str("skipinitialspace", py_bool(dialect.skipinitialspace));
                ddict.insert_str("lineterminator", py_str(&dialect.lineterminator));
                ddict.insert_str("quoting", py_int(dialect.quoting));
                ddict.insert_str("strict", py_bool(dialect.strict));
                let mut type_dict = StdHashMap::new();
            type_dict.insert_str("__setattr__", PyObjectRef::new(PyObject::BuiltinFunction{ name: "__setattr__".to_string(), func: |_| Err(PyError::AttributeError("attribute is read-only".to_string())) }));
            type_dict.insert_str("__delattr__", PyObjectRef::new(PyObject::BuiltinFunction{ name: "__delattr__".to_string(), func: |_| Err(PyError::AttributeError("attribute is read-only".to_string())) }));
            let dtype = PyObjectRef::new(PyObject::Type{ name: "Dialect".to_string(), dict: Box::new(str_map_to_typedict(type_dict)), bases: vec![], mro: vec![] });
                PyObjectRef::new(PyObject::Instance{ typ: dtype, dict: ddict })
            }).ok();
            Ok(py_none())
        }}));
        dict.insert_str("writeheader", PyObjectRef::new(PyObject::BuiltinFunction{ name: "writeheader".to_string(), func: |args| {
            let self_obj = &args[0];
            let fieldnames = self_obj.borrow().get_attribute("fieldnames").and_then(|v| {
                let b=v.borrow();
                if let PyObject::List(items)=&*b { Ok(items.clone()) } else { Err(PyError::type_error("fieldnames")) }
            }).unwrap_or_default();
            let header_dict = crate::object::py_dict();
            {
                let mut hb = header_dict.borrow_mut();
                if let PyObject::Dict(d) = &mut *hb {
                    for f in &fieldnames { d.set(f.clone(), f.clone()).ok(); }
                }
            }
            let wrow = self_obj.borrow().get_attribute("writerow").unwrap();
            let ret = crate::object::call_function_disposable(&wrow, vec![self_obj.clone(), header_dict.clone()], vec![])?;
            Ok(ret)
        }}));
        dict.insert_str("writerow", PyObjectRef::new(PyObject::BuiltinFunction{ name: "writerow".to_string(), func: |args| {
            if args.len()<2 { return Err(PyError::type_error("writerow() missing argument")); }
            let self_obj = &args[0];
            let rowdict = &args[1];
            let fieldnames = self_obj.borrow().get_attribute("fieldnames").and_then(|v| {
                let b=v.borrow();
                if let PyObject::List(items)=&*b { Ok(items.clone()) } else { Err(PyError::type_error("fieldnames")) }
            }).unwrap_or_default();
            let extrasaction = self_obj.borrow().get_attribute("extrasaction").map(|v| v.str()).unwrap_or_else(|_| "raise".to_string());
            if extrasaction=="raise" {
                let dict_b = rowdict.borrow();
                if let PyObject::Dict(d) = &*dict_b {
                    let keys: Vec<String> = d.items().iter().map(|(k,_)| k.str()).collect();
                    let field_set: StdHashMap<String, bool> = fieldnames.iter().map(|f| (f.str(), true)).collect();
                    let mut wrong = Vec::new();
                    for k in keys { if !field_set.contains_key(&k) { wrong.push(k); } }
                    if !wrong.is_empty() {
                        return Err(PyError::value_error(format!("dict contains fields not in fieldnames: {}", wrong.iter().map(|s| format!("'{}'", s)).collect::<Vec<_>>().join(", "))));
                    }
                }
            }
            let restval = self_obj.borrow().get_attribute("restval").unwrap_or_else(|_| py_str(""));
            let mut cells: Vec<PyObjectRef> = Vec::new();
            for fname in &fieldnames {
                let key = fname.str();
                let val = if let PyObject::Dict(d) = &*rowdict.borrow() {
                    d.get(&py_str(&key)).ok().flatten().unwrap_or_else(|| restval.clone())
                } else {
                    rowdict.borrow().get_attribute(&key).unwrap_or_else(|_| restval.clone())
                };
                cells.push(val);
            }
            let dialect_obj = self_obj.borrow().get_attribute("_dialect").unwrap();
            let dialect = {
                let db = dialect_obj.borrow();
                if let PyObject::Instance{ dict, .. } = &*db {
                    let delim = dict.get("delimiter").map(|v| v.str().chars().next().unwrap_or(',')).unwrap_or(',');
                    let qc = dict.get("quotechar").and_then(|v| if matches!(&*v.borrow(), PyObject::None) { None } else { Some(v.str().chars().next().unwrap()) });
                    let ec = dict.get("escapechar").and_then(|v| if matches!(&*v.borrow(), PyObject::None) { None } else { Some(v.str().chars().next().unwrap()) });
                    let dq = dict.get("doublequote").map(|v| v.truthy()).unwrap_or(true);
                    let sis = dict.get("skipinitialspace").map(|v| v.truthy()).unwrap_or(false);
                    let lt = dict.get("lineterminator").map(|v| v.str()).unwrap_or_else(|| "\r\n".to_string());
                    let q = dict.get("quoting").and_then(|v| v.as_i64()).unwrap_or(0);
                    let strict = dict.get("strict").map(|v| v.truthy()).unwrap_or(false);
                    CsvDialect{ delimiter: delim, quotechar: qc, escapechar: ec, doublequote: dq, skipinitialspace: sis, lineterminator: lt, quoting: q, strict }
                } else { CsvDialect::excel() }
            };
            let out = format_csv_row(cells, &dialect)?;
            let fobj = self_obj.borrow().get_attribute("_fileobj").unwrap();
            let actual_fobj = fobj.borrow().get_attribute("_file").unwrap_or_else(|_| fobj.clone());
            let write_res = crate::object::with_vm_mut(|vm| crate::object::call_method_rebound(vm, &actual_fobj, "write", vec![py_str(&out)]));
            match write_res {
                Ok(Ok(_)) => {},
                Ok(Err(e)) => return Err(e),
                Err(e) => return Err(e),
            }
            Ok(py_int(out.len() as i64))
        }}));
        dict.insert_str("writerows", PyObjectRef::new(PyObject::BuiltinFunction{ name: "writerows".to_string(), func: |args| {
            if args.len()<2 { return Err(PyError::type_error("writerows() missing argument")); }
            let self_obj = &args[0];
            let rows_arg = &args[1];
            let iter = crate::object::builtin_iter(&[rows_arg.clone()])?;
            let wrow = self_obj.borrow().get_attribute("writerow").unwrap();
            loop {
                match crate::object::builtin_next(&[iter.clone()]) {
                    Ok(row) => { crate::object::call_function_disposable(&wrow, vec![self_obj.clone(), row.clone()], vec![])?; },
                    Err(PyError::StopIteration) => break,
                    Err(e) => return Err(e),
                }
            }
            Ok(py_none())
        }}));
    }
    d.insert("DictWriter".to_string(), dict_writer_type.clone());

    // Helper for sniffer
    fn sniff_guess_quote_and_delimiter(sample: &str, delimiters: Option<&str>) -> (String, bool, Option<char>, bool) {
        let norm = sample.replace("\r\n", "\n").replace('\r', "\n");
        let candidates: Vec<char> = if let Some(d) = delimiters {
            d.chars().collect()
        } else {
            vec![',', '\t', ';', ' ', ':', '|', '+', '\0', '?', '/', '"', '\'']
        };
        let quote_candidates = ['"', '\''];
        let mut best_quote: Option<char> = None;
        let mut best_delim: Option<char> = None;
        let mut best_count = 0;
        for &qc in &quote_candidates {
            for &dc in &candidates {
                if qc == dc { continue; }
                let pat1 = format!("{}{}", dc, qc);
                let pat2 = format!("{}{}", qc, dc);
                let cnt1 = norm.matches(&pat1).count();
                let cnt2 = norm.matches(&pat2).count();
                let total = cnt1 + cnt2;
                if total > best_count {
                    best_count = total;
                    best_quote = Some(qc);
                    best_delim = Some(dc);
                }
            }
        }
        if best_count == 0 {
            // Check for sample that starts and ends with quote and contains delimiter inside (e.g. "'123;4'" with delimiter ';' and quote "'")
            for &qc in &quote_candidates {
                for &dc in &candidates {
                    if qc == dc { continue; }
                    if norm.len() >= 2 && norm.starts_with(qc) && norm.ends_with(qc) {
                        let inner = &norm[1..norm.len()-1];
                        if inner.contains(dc) {
                            if let Some(d) = delimiters {
                                if !d.contains(dc) { continue; }
                            }
                            let dq_str = format!("{}{}", qc, qc);
                            let doublequote = norm.contains(&dq_str);
                            let delim_count = norm.matches(dc).count();
                            let delim_space = norm.matches(format!("{} ", dc).as_str()).count();
                            let skip = delim_count > 0 && delim_count == delim_space;
                            return (qc.to_string(), doublequote, Some(dc), skip);
                        }
                    }
                }
            }
            return ("".to_string(), false, None, false);
        }
        let qc = best_quote.unwrap();
        let dc = best_delim.unwrap();
        // check delimiters param restriction
        if let Some(d) = delimiters {
            if !d.contains(dc) {
                return ("".to_string(), false, None, false);
            }
        }
        // determine doublequote
        let dq_str = format!("{}{}", qc, qc);
        let doublequote = norm.contains(&dq_str);
        // skipinitialspace: check if delim followed by space count equals delim count
        let delim_count = norm.matches(dc).count();
        let delim_space = norm.matches(format!("{} ", dc).as_str()).count();
        let skip = delim_count > 0 && delim_count == delim_space;
        (qc.to_string(), doublequote, Some(dc), skip)
    }
    fn sniff_guess_delimiter(sample: &str, delimiters: Option<&str>) -> (Option<char>, bool) {
        let norm = sample.replace("\r\n", "\n").replace('\r', "\n");
        let lines: Vec<&str> = norm.split('\n').filter(|s| !s.is_empty()).collect();
        if lines.is_empty() { return (None, false); }
        let candidates: Vec<char> = if let Some(d) = delimiters {
            d.chars().collect()
        } else {
            vec![',', '\t', ';', ' ', ':', '|', '+', '\0', '?', '/', '-', '_', '.', '#', '@', '!']
        };
        // Build char frequency per line and evaluate consistency similar to CPython
        let mut best: Option<char> = None;
        let mut best_consistency = 0.0;
        let mut best_total = 0usize;
        let preferred = vec![',', '\t', ';', ' ', ':'];
        let total_lines = lines.len() as f64;
        for &ch in &candidates {
            // count per line
            let mut counts: Vec<usize> = lines.iter().map(|l| l.matches(ch).count()).collect();
            if counts.iter().all(|&c| c == 0) { continue; }
            // find mode
            let mut freq_map: StdHashMap<usize, usize> = StdHashMap::new();
            for &c in &counts { *freq_map.entry(c).or_insert(0) += 1; }
            let (mode, mode_cnt) = freq_map.into_iter().max_by_key(|(_, v)| *v).unwrap();
            if mode == 0 { continue; }
            let consistency = mode_cnt as f64 / total_lines;
            if consistency < 0.9 { continue; }
            let total: usize = counts.iter().sum();
            // Prefer higher consistency, then higher total, then preferred order
            if consistency > best_consistency || (consistency == best_consistency && total > best_total) {
                best = Some(ch);
                best_consistency = consistency;
                best_total = total;
            }
        }
        if let Some(ch) = best {
            // check skipinitialspace
            let delim_count = norm.matches(ch).count();
            let delim_space = norm.matches(format!("{} ", ch).as_str()).count();
            let skip = delim_count > 0 && delim_count == delim_space;
            return (Some(ch), skip);
        }
        // Fallback to max total count among candidates that appear
        let mut max_ch: Option<char> = None;
        let mut max_cnt = 0;
        for &ch in &candidates {
            let cnt = norm.matches(ch).count();
            if cnt > max_cnt { max_cnt = cnt; max_ch = Some(ch); }
        }
        if let Some(ch) = max_ch {
            if max_cnt == 0 { return (None, false); }
            let delim_space = norm.matches(format!("{} ", ch).as_str()).count();
            let skip = max_cnt == delim_space;
            // For preferred handling when multiple with same count, choose preferred
            // If we have multiple candidates with same max, pick preferred
            let mut candidates_with_max: Vec<char> = candidates.iter().cloned().filter(|&c| norm.matches(c).count() == max_cnt).collect();
            if candidates_with_max.len() > 1 {
                for &p in &preferred {
                    if candidates_with_max.contains(&p) {
                        let ch2 = p;
                        let skip2 = norm.matches(ch2).count() == norm.matches(format!("{} ", ch2).as_str()).count();
                        return (Some(ch2), skip2);
                    }
                }
            }
            return (Some(ch), skip);
        }
        (None, false)
    }
    fn csv_has_header(sample: &str) -> PyResult<bool> {
        // Use sniff to get dialect, then parse sample
        let (quotechar_str, doublequote, delim_opt, skip) = sniff_guess_quote_and_delimiter(sample, None);
        let (delim2, skip2) = sniff_guess_delimiter(sample, None);
        let delim = if let Some(d) = delim_opt { d } else { delim2.unwrap_or(',') };
        let skipinitial = if delim_opt.is_some() { skip } else { skip2 };
        let quotechar = if quotechar_str.is_empty() { Some('"') } else { quotechar_str.chars().next() };
        let dialect = CsvDialect { delimiter: delim, quotechar, escapechar: None, doublequote, skipinitialspace: skipinitial, lineterminator: "\r\n".to_string(), quoting: 0, strict: false };
        // If sniff failed to find delimiter, try sniff again with default
        let lines_raw: Vec<String> = sample.lines().map(|s| s.to_string()).collect();
        // Actually use parse_csv_lines to get rows
        let rows = parse_csv_lines(lines_raw.clone(), &dialect).unwrap_or_else(|_| vec![]);
        if rows.is_empty() { return Ok(false); }
        let header = &rows[0];
        let columns = header.len();
        if columns == 0 { return Ok(false); }
        let mut column_types: StdHashMap<usize, Option<String>> = StdHashMap::new();
        for i in 0..columns { column_types.insert(i, None); }
        let mut checked = 0;
        for row in rows.iter().skip(1) {
            if checked > 20 { break; }
            checked += 1;
            if row.len() != columns { continue; }
            for col in 0..columns {
                if !column_types.contains_key(&col) { continue; }
                let val_str = row[col].str();
                // try to determine type: attempt complex/float
                let this_type = if val_str.parse::<f64>().is_ok() || val_str.contains('j') || val_str.contains('J') {
                    "complex".to_string()
                } else {
                    format!("len:{}", val_str.len())
                };
                if let Some(entry) = column_types.get_mut(&col) {
                    if entry.is_none() {
                        *entry = Some(this_type);
                    } else if entry.as_ref().unwrap() != &this_type {
                        // inconsistent, remove
                        column_types.remove(&col);
                    }
                }
            }
            // need to handle removal during iteration - we collected keys beforehand
            // Simplify: recreate map without inconsistent? For now we just remove as we go, but iteration over 0..columns will skip removed?
            // We use a separate list of keys to check
        }
        // Actually we need to iterate over copy of keys
        // Re-evaluate with proper removal: we did above but may miss some
        // For simplicity, redo with proper logic: we will use a mutable map and check each column
        // To avoid borrow issues, we already did, but removal during loop over 0..columns is okay if we check existence.
        // Now vote
        let mut has_header = 0i32;
        for (col, typ_opt) in column_types {
            if let Some(typ) = typ_opt {
                if typ.starts_with("len:") {
                    let len_val: usize = typ[4..].parse().unwrap_or(0);
                    if header[col].str().len() != len_val {
                        has_header += 1;
                    } else {
                        has_header -= 1;
                    }
                } else {
                    // complex type
                    let hdr = header[col].str();
                    if hdr.parse::<f64>().is_ok() || hdr.contains('j') || hdr.contains('J') {
                        has_header -= 1;
                    } else {
                        has_header += 1;
                    }
                }
            }
        }
        Ok(has_header > 0)
    }
    let sniffer_type = PyObjectRef::new(PyObject::Type{ name: "Sniffer".to_string(), dict: Box::new(str_map_to_typedict({
        let mut m = StdHashMap::new();
        m.insert_str("__init__", PyObjectRef::new(PyObject::BuiltinFunction{ name: "__init__".to_string(), func: |_| Ok(py_none()) }));
        m.insert_str("sniff", PyObjectRef::new(PyObject::BuiltinFunction{ name: "sniff".to_string(), func: |args| {
            if args.len()<2 { return Err(PyError::type_error("sniff() missing required argument: 'sample'")); }
            let sample = args[1].str();
            let delims = if args.len()>2 && !matches!(&*args[2].borrow(), PyObject::None) { Some(args[2].str()) } else { None };
            let sample_norm = sample.replace("\r\n", "\n").replace('\r', "\n");
            let (qc_str, dq, delim_opt, skip) = sniff_guess_quote_and_delimiter(&sample_norm, delims.as_deref());
            let (delim2, skip2) = sniff_guess_delimiter(&sample_norm, delims.as_deref());
            let delim = if let Some(d) = delim_opt { Some(d) } else { delim2 };
            if delim.is_none() {
                return Err(dialect_error("Could not determine delimiter".to_string()));
            }
            let dch = delim.unwrap();
            let qch = if qc_str.is_empty() { '"' } else { qc_str.chars().next().unwrap() };
            let dq_val = if qc_str.is_empty() { false } else { dq };
            let skip_val = if delim_opt.is_some() { skip } else { skip2 };
            // If sniff found quote but not delim, use delim2
            // Build dialect
            let mut dict = crate::object::AttrMap::new();
            dict.insert_str("delimiter", py_str(&dch.to_string()));
            dict.insert_str("quotechar", py_str(&qch.to_string()));
            dict.insert_str("escapechar", py_none());
            dict.insert_str("doublequote", py_bool(dq_val));
            dict.insert_str("skipinitialspace", py_bool(skip_val));
            dict.insert_str("lineterminator", py_str("\r\n"));
            dict.insert_str("quoting", py_int(0));
            dict.insert_str("strict", py_bool(false));
            let mut type_dict = StdHashMap::new();
            type_dict.insert_str("__setattr__", PyObjectRef::new(PyObject::BuiltinFunction{ name: "__setattr__".to_string(), func: |_| Err(PyError::AttributeError("attribute is read-only".to_string())) }));
            type_dict.insert_str("__delattr__", PyObjectRef::new(PyObject::BuiltinFunction{ name: "__delattr__".to_string(), func: |_| Err(PyError::AttributeError("attribute is read-only".to_string())) }));
            type_dict.insert_str("__copy__", PyObjectRef::new(PyObject::BuiltinFunction{ name: "__copy__".to_string(), func: |_| Err(PyError::type_error("cannot pickle 'Dialect' instances")) }));
            type_dict.insert_str("__deepcopy__", PyObjectRef::new(PyObject::BuiltinFunction{ name: "__deepcopy__".to_string(), func: |_| Err(PyError::type_error("cannot pickle 'Dialect' instances")) }));
            type_dict.insert_str("__reduce__", PyObjectRef::new(PyObject::BuiltinFunction{ name: "__reduce__".to_string(), func: |_| Err(PyError::type_error("cannot pickle 'Dialect' instances")) }));
            type_dict.insert_str("__reduce_ex__", PyObjectRef::new(PyObject::BuiltinFunction{ name: "__reduce_ex__".to_string(), func: |_| Err(PyError::type_error("cannot pickle 'Dialect' instances")) }));
            let dtype = PyObjectRef::new(PyObject::Type{ name: "Dialect".to_string(), dict: Box::new(str_map_to_typedict(type_dict)), bases: vec![], mro: vec![] });
            Ok(PyObjectRef::new(PyObject::Instance{ typ: dtype, dict }))
        }}));
        m.insert_str("has_header", PyObjectRef::new(PyObject::BuiltinFunction{ name: "has_header".to_string(), func: |args| {
            if args.len()<2 { return Err(PyError::type_error("has_header() missing required argument")); }
            let sample = args[1].str();
            let res = csv_has_header(&sample)?;
            Ok(py_bool(res))
        }}));
        m
    })), bases: vec![], mro: vec![] });
    d.insert("Sniffer".to_string(), sniffer_type);

    d.insert("__all__".to_string(), py_list(vec![py_str("QUOTE_MINIMAL"), py_str("QUOTE_ALL"), py_str("QUOTE_NONNUMERIC"), py_str("QUOTE_NONE"), py_str("QUOTE_STRINGS"), py_str("QUOTE_NOTNULL"), py_str("Error"), py_str("Dialect"), py_str("excel"), py_str("excel_tab"), py_str("field_size_limit"), py_str("reader"), py_str("writer"), py_str("register_dialect"), py_str("get_dialect"), py_str("list_dialects"), py_str("Sniffer"), py_str("unregister_dialect"), py_str("DictReader"), py_str("DictWriter"), py_str("unix_dialect")]));
    d
}

