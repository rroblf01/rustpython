use crate::object::*;
use std::collections::HashMap;
use once_cell::sync::Lazy;

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

    // Extracts `width` from either a positional 2nd argument OR a
    // `width=` keyword (arriving as this project's own trailing packed-
    // kwargs-dict convention) — real code calls `textwrap.fill(text,
    // width=30)` far more often than the purely-positional form, which
    // this function's own PREVIOUS version (`args[2]`-only) completely
    // missed, silently falling back to the default 70 regardless of what
    // was actually requested.
    fn extract_width(args: &[PyObjectRef]) -> usize {
        if let Some(kwargs) = args.last().and_then(|a| if let PyObject::Dict(d) = &*a.borrow() { Some(d.clone()) } else { None }) {
            if let Some(w) = kwargs.get(&py_str("width")).ok().flatten().and_then(|v| v.as_i64()) {
                return w as usize;
            }
        }
        args.get(1).and_then(|v| v.as_i64()).map(|w| w as usize).unwrap_or(70)
    }

    tw_func!("fill", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("fill() takes at least 1 argument"));
        }
        let text = args[0].str();
        let width = extract_width(args);
        Ok(py_str(&wrap_lines(&text, width, "", "").join("\n")))
    });

    // Shared word-wrap core used by both `fill()` (joins with `\n`, already
    // existed above) and the new `wrap()`/`TextWrapper.wrap()` (returns the
    // list of lines directly — the more fundamental of the two operations
    // in real `textwrap`, `fill()` is literally defined as `'\n'.join(wrap(...))`).
    fn wrap_lines(text: &str, width: usize, initial_indent: &str, subsequent_indent: &str) -> Vec<String> {
        if width == 0 {
            return vec![text.to_string()];
        }
        let words: Vec<&str> = text.split_whitespace().collect();
        let mut lines: Vec<String> = Vec::new();
        let mut current = String::new();
        for word in words {
            let indent = if lines.is_empty() { initial_indent } else { subsequent_indent };
            if current.is_empty() {
                current = format!("{}{}", indent, word);
            } else if current.len() + 1 + word.len() <= width {
                current.push(' ');
                current.push_str(word);
            } else {
                lines.push(current);
                let indent = subsequent_indent;
                current = format!("{}{}", indent, word);
            }
        }
        if !current.is_empty() {
            lines.push(current);
        }
        lines
    }

    // `textwrap.wrap(text, width=70)` — was missing entirely, even though
    // `fill()` (which just joins `wrap()`'s own result with `\n`) already
    // existed; real code very commonly wants the individual lines as a
    // list (e.g. to prefix each with `"> "` for a quoted reply, or to count
    // lines) rather than a single joined string.
    tw_func!("wrap", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("wrap() takes at least 1 argument"));
        }
        let text = args[0].str();
        let width = extract_width(args);
        let lines = wrap_lines(&text, width, "", "");
        Ok(py_list(lines.into_iter().map(|l| py_str(&l)).collect()))
    });

    // `textwrap.TextWrapper` — the real, OOP-configurable counterpart to
    // the plain `wrap()`/`fill()` functions (lets code set `width`/
    // `initial_indent`/`subsequent_indent` once and reuse across many
    // calls) — was missing entirely. A synthetic native `Type` (same
    // pattern as `time.struct_time`/`platform.uname_result` elsewhere in
    // this project) storing its config as instance attributes, with real
    // `.wrap(text)`/`.fill(text)` methods reading them back.
    {
        let mut type_dict: HashMap<String, PyObjectRef> = HashMap::new();
        type_dict.insert_str("__init__", PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__init__".to_string(),
            func: |args| {
                if args.is_empty() { return Err(PyError::type_error("__init__() missing self")); }
                let kwargs = args.last().and_then(|a| if let PyObject::Dict(d) = &*a.borrow() { Some(d.clone()) } else { None });
                let get_kw = |name: &str| kwargs.as_ref().and_then(|d| d.get(&py_str(name)).ok().flatten());
                let width = get_kw("width").and_then(|v| v.as_i64()).unwrap_or(70);
                let initial_indent = get_kw("initial_indent").map(|v| v.str()).unwrap_or_default();
                let subsequent_indent = get_kw("subsequent_indent").map(|v| v.str()).unwrap_or_default();
                if let PyObject::Instance { dict, .. } = &mut *args[0].borrow_mut() {
                    dict.insert_str("width", py_int(width));
                    dict.insert_str("initial_indent", py_str(&initial_indent));
                    dict.insert_str("subsequent_indent", py_str(&subsequent_indent));
                }
                Ok(py_none())
            },
        }));
        type_dict.insert_str("wrap", PyObjectRef::new(PyObject::BuiltinFunction {
            name: "wrap".to_string(),
            func: |args| {
                if args.len() < 2 { return Err(PyError::type_error("wrap() takes exactly 1 argument")); }
                let (width, ii, si) = if let PyObject::Instance { dict, .. } = &*args[0].borrow() {
                    (
                        dict.get_str("width").and_then(|v| v.as_i64()).unwrap_or(70) as usize,
                        dict.get_str("initial_indent").map(|v| v.str()).unwrap_or_default(),
                        dict.get_str("subsequent_indent").map(|v| v.str()).unwrap_or_default(),
                    )
                } else { (70, String::new(), String::new()) };
                let text = args[1].str();
                let lines = wrap_lines(&text, width, &ii, &si);
                Ok(py_list(lines.into_iter().map(|l| py_str(&l)).collect()))
            },
        }));
        type_dict.insert_str("fill", PyObjectRef::new(PyObject::BuiltinFunction {
            name: "fill".to_string(),
            func: |args| {
                if args.len() < 2 { return Err(PyError::type_error("fill() takes exactly 1 argument")); }
                let (width, ii, si) = if let PyObject::Instance { dict, .. } = &*args[0].borrow() {
                    (
                        dict.get_str("width").and_then(|v| v.as_i64()).unwrap_or(70) as usize,
                        dict.get_str("initial_indent").map(|v| v.str()).unwrap_or_default(),
                        dict.get_str("subsequent_indent").map(|v| v.str()).unwrap_or_default(),
                    )
                } else { (70, String::new(), String::new()) };
                let text = args[1].str();
                let lines = wrap_lines(&text, width, &ii, &si);
                Ok(py_str(&lines.join("\n")))
            },
        }));
        d.insert_str("TextWrapper", PyObjectRef::new(PyObject::Type {
            name: "TextWrapper".to_string(), dict: Box::new(str_map_to_typedict(type_dict)), bases: vec![], mro: vec![],
        }));
    }

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

    d.insert_str("ascii_letters", py_str(ascii_letters));
    d.insert_str("ascii_lowercase", py_str(ascii_lowercase));
    d.insert_str("ascii_uppercase", py_str(ascii_uppercase));
    d.insert_str("digits", py_str(digits));
    d.insert_str("hexdigits", py_str(hexdigits));
    d.insert_str("octdigits", py_str(octdigits));
    d.insert_str("punctuation", py_str(punctuation));
    d.insert_str("printable", py_str(printable));
    d.insert_str("whitespace", py_str(whitespace));

    d
}

pub fn create_reprlib_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    d.insert_str("repr", PyObjectRef::new(PyObject::BuiltinFunction {
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

// Moved here from object.rs (was under a "=== MIMETYPES MODULE ===" banner
// in the monolithic object.rs — see the file-splitting refactor's memory
// entry for context).
// Static MIME type database: extension -> (type, encoding)
static KNOWN_TYPES: Lazy<HashMap<String, (String, String)>> = Lazy::new(|| {
    HashMap::from([
        (".html".to_string(), ("text/html".to_string(), "".to_string())),
        (".htm".to_string(), ("text/html".to_string(), "".to_string())),
        (".css".to_string(), ("text/css".to_string(), "".to_string())),
        (".js".to_string(), ("application/javascript".to_string(), "".to_string())),
        (".json".to_string(), ("application/json".to_string(), "".to_string())),
        (".xml".to_string(), ("application/xml".to_string(), "".to_string())),
        (".txt".to_string(), ("text/plain".to_string(), "".to_string())),
        (".csv".to_string(), ("text/csv".to_string(), "".to_string())),
        (".md".to_string(), ("text/markdown".to_string(), "".to_string())),
        (".py".to_string(), ("text/x-python".to_string(), "".to_string())),
        (".png".to_string(), ("image/png".to_string(), "".to_string())),
        (".jpg".to_string(), ("image/jpeg".to_string(), "".to_string())),
        (".jpeg".to_string(), ("image/jpeg".to_string(), "".to_string())),
        (".gif".to_string(), ("image/gif".to_string(), "".to_string())),
        (".bmp".to_string(), ("image/bmp".to_string(), "".to_string())),
        (".ico".to_string(), ("image/x-icon".to_string(), "".to_string())),
        (".svg".to_string(), ("image/svg+xml".to_string(), "".to_string())),
        (".webp".to_string(), ("image/webp".to_string(), "".to_string())),
        (".mp3".to_string(), ("audio/mpeg".to_string(), "".to_string())),
        (".wav".to_string(), ("audio/wav".to_string(), "".to_string())),
        (".ogg".to_string(), ("audio/ogg".to_string(), "".to_string())),
        (".mp4".to_string(), ("video/mp4".to_string(), "".to_string())),
        (".webm".to_string(), ("video/webm".to_string(), "".to_string())),
        (".avi".to_string(), ("video/x-msvideo".to_string(), "".to_string())),
        (".mov".to_string(), ("video/quicktime".to_string(), "".to_string())),
        (".pdf".to_string(), ("application/pdf".to_string(), "".to_string())),
        (".zip".to_string(), ("application/zip".to_string(), "".to_string())),
        (".gz".to_string(), ("application/gzip".to_string(), "".to_string())),
        (".tar".to_string(), ("application/x-tar".to_string(), "".to_string())),
        (".rar".to_string(), ("application/vnd.rar".to_string(), "".to_string())),
        (".7z".to_string(), ("application/x-7z-compressed".to_string(), "".to_string())),
        (".exe".to_string(), ("application/x-msdownload".to_string(), "".to_string())),
        (".bin".to_string(), ("application/octet-stream".to_string(), "".to_string())),
        (".wasm".to_string(), ("application/wasm".to_string(), "".to_string())),
        (".woff".to_string(), ("font/woff".to_string(), "".to_string())),
        (".woff2".to_string(), ("font/woff2".to_string(), "".to_string())),
        (".ttf".to_string(), ("font/ttf".to_string(), "".to_string())),
        (".otf".to_string(), ("font/otf".to_string(), "".to_string())),
        (".yaml".to_string(), ("text/yaml".to_string(), "".to_string())),
        (".yml".to_string(), ("text/yaml".to_string(), "".to_string())),
        (".toml".to_string(), ("application/toml".to_string(), "".to_string())),
        (".doc".to_string(), ("application/msword".to_string(), "".to_string())),
        (".docx".to_string(), ("application/vnd.openxmlformats-officedocument.wordprocessingml.document".to_string(), "".to_string())),
        (".xls".to_string(), ("application/vnd.ms-excel".to_string(), "".to_string())),
        (".xlsx".to_string(), ("application/vnd.openxmlformats-officedocument.spreadsheetml.sheet".to_string(), "".to_string())),
        (".ppt".to_string(), ("application/vnd.ms-powerpoint".to_string(), "".to_string())),
        (".pptx".to_string(), ("application/vnd.openxmlformats-officedocument.presentationml.presentation".to_string(), "".to_string())),
        (".rtf".to_string(), ("application/rtf".to_string(), "".to_string())),
    ])
});

// Static reverse mapping: type -> extension
static KNOWN_EXTS: Lazy<HashMap<String, String>> = Lazy::new(|| {
    HashMap::from([
        ("text/html".to_string(), ".html".to_string()),
        ("text/css".to_string(), ".css".to_string()),
        ("application/javascript".to_string(), ".js".to_string()),
        ("application/json".to_string(), ".json".to_string()),
        ("application/xml".to_string(), ".xml".to_string()),
        ("text/plain".to_string(), ".txt".to_string()),
        ("text/csv".to_string(), ".csv".to_string()),
        ("text/markdown".to_string(), ".md".to_string()),
        ("text/x-python".to_string(), ".py".to_string()),
        ("image/png".to_string(), ".png".to_string()),
        ("image/jpeg".to_string(), ".jpg".to_string()),
        ("image/gif".to_string(), ".gif".to_string()),
        ("image/bmp".to_string(), ".bmp".to_string()),
        ("image/x-icon".to_string(), ".ico".to_string()),
        ("image/svg+xml".to_string(), ".svg".to_string()),
        ("image/webp".to_string(), ".webp".to_string()),
        ("audio/mpeg".to_string(), ".mp3".to_string()),
        ("audio/wav".to_string(), ".wav".to_string()),
        ("audio/ogg".to_string(), ".ogg".to_string()),
        ("video/mp4".to_string(), ".mp4".to_string()),
        ("video/webm".to_string(), ".webm".to_string()),
        ("video/x-msvideo".to_string(), ".avi".to_string()),
        ("video/quicktime".to_string(), ".mov".to_string()),
        ("application/pdf".to_string(), ".pdf".to_string()),
        ("application/zip".to_string(), ".zip".to_string()),
        ("application/gzip".to_string(), ".gz".to_string()),
        ("application/x-tar".to_string(), ".tar".to_string()),
        ("application/vnd.rar".to_string(), ".rar".to_string()),
        ("application/x-7z-compressed".to_string(), ".7z".to_string()),
        ("application/x-msdownload".to_string(), ".exe".to_string()),
        ("application/octet-stream".to_string(), ".bin".to_string()),
        ("application/wasm".to_string(), ".wasm".to_string()),
        ("font/woff".to_string(), ".woff".to_string()),
        ("font/woff2".to_string(), ".woff2".to_string()),
        ("font/ttf".to_string(), ".ttf".to_string()),
        ("font/otf".to_string(), ".otf".to_string()),
        ("text/yaml".to_string(), ".yaml".to_string()),
        ("application/toml".to_string(), ".toml".to_string()),
        ("application/msword".to_string(), ".doc".to_string()),
        ("application/vnd.openxmlformats-officedocument.wordprocessingml.document".to_string(), ".docx".to_string()),
        ("application/vnd.ms-excel".to_string(), ".xls".to_string()),
        ("application/vnd.openxmlformats-officedocument.spreadsheetml.sheet".to_string(), ".xlsx".to_string()),
        ("application/vnd.ms-powerpoint".to_string(), ".ppt".to_string()),
        ("application/vnd.openxmlformats-officedocument.presentationml.presentation".to_string(), ".pptx".to_string()),
        ("application/rtf".to_string(), ".rtf".to_string()),
    ])
});

pub fn mime_guess_type(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error("guess_type() takes at least 1 argument"));
    }
    let url = args[0].str();
    // Strip query string and fragment
    let path = url.split('?').next().unwrap_or("").split('#').next().unwrap_or("");
    let ext = {
        let p = path.rfind('.').map(|i| &path[i..]).unwrap_or("");
        p.to_lowercase()
    };
    let (mime_type, encoding) = KNOWN_TYPES.get(&ext).cloned().unwrap_or_else(|| {
        ("application/octet-stream".to_string(), "".to_string())
    });
    let encoding = if encoding.is_empty() { py_none() } else { py_str(&encoding) };
    let result = PyObjectRef::new(PyObject::Tuple(vec![py_str(&mime_type), encoding]));
    Ok(result)
}

pub fn mime_guess_extension(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error("guess_extension() takes at least 1 argument"));
    }
    let mime_type = args[0].str();
    let ext = KNOWN_EXTS.get(&mime_type);
    match ext {
        Some(e) => Ok(py_str(e)),
        None => Ok(py_none()),
    }
}

pub fn mime_add_type(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 2 {
        return Err(PyError::type_error("add_type() takes at least 2 arguments (type, ext)"));
    }
    let _ = args;
    Ok(py_none())
}

pub fn create_mimetypes_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    d.insert_str("guess_type", PyObjectRef::new(PyObject::BuiltinFunction { name: "guess_type".to_string(), func: mime_guess_type }));
    d.insert_str("guess_extension", PyObjectRef::new(PyObject::BuiltinFunction { name: "guess_extension".to_string(), func: mime_guess_extension }));
    d.insert_str("add_type", PyObjectRef::new(PyObject::BuiltinFunction { name: "add_type".to_string(), func: mime_add_type }));
    // list of known types, init, read_mime_types, etc. can be added as needed
    d.insert_str("known_types", py_dict());
    d.insert_str("inited", py_bool(false));
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
            let mut dict = AttrMap::new();

            dict.insert_str("vformat", PyObjectRef::new(PyObject::BuiltinFunction {
                name: "vformat".to_string(),
                func: |_| Ok(py_str("vformat stub")),
            }));

            dict.insert_str("format", PyObjectRef::new(PyObject::BuiltinFunction {
                name: "format".to_string(),
                func: |fargs| {
                    if fargs.is_empty() { return Ok(py_str("")); }
                    Ok(py_str(&fargs[0].str()))
                },
            }));

            dict.insert_str("parse", PyObjectRef::new(PyObject::BuiltinFunction {
                name: "parse".to_string(),
                func: |_| Ok(py_list(vec![])),
            }));

            dict.insert_str("get_field", PyObjectRef::new(PyObject::BuiltinFunction {
                name: "get_field".to_string(),
                func: |_| Ok(py_str("")),
            }));

            dict.insert_str("get_value", PyObjectRef::new(PyObject::BuiltinFunction {
                name: "get_value".to_string(),
                func: |_| Ok(py_str("")),
            }));

            dict.insert_str("check_unused_args", PyObjectRef::new(PyObject::BuiltinFunction {
                name: "check_unused_args".to_string(),
                func: |_| Ok(py_none()),
            }));

            dict.insert_str("format_field", PyObjectRef::new(PyObject::BuiltinFunction {
                name: "format_field".to_string(),
                func: |fargs| {
                    if fargs.is_empty() { return Ok(py_str("")); }
                    Ok(py_str(&fargs[0].str()))
                },
            }));

            dict.insert_str("convert_field", PyObjectRef::new(PyObject::BuiltinFunction {
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

    d.insert_str("Formatter", formatter);
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
            let mut dict = AttrMap::new();
            dict.insert_str("ratio", PyObjectRef::new(PyObject::BuiltinFunction {
                name: "ratio".to_string(),
                func: |_| Ok(py_float(1.0)),
            }));
            Ok(PyObjectRef::new(PyObject::Instance {
                typ: py_str("SequenceMatcher"),
                dict,
            }))
        },
    });
    d.insert_str("SequenceMatcher", seq_matcher);

    dfl_func!("get_close_matches", |args| {
        let _word = if args.len() > 0 { args[0].str() } else { return Err(PyError::type_error("get_close_matches() requires at least 1 argument")); };
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
            let mut dict = AttrMap::new();

            // feed(data) — accumulates data
            dict.insert_str("feed", PyObjectRef::new(PyObject::BuiltinFunction {
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
            dict.insert_str("close", PyObjectRef::new(PyObject::BuiltinFunction {
                name: "close".to_string(),
                func: |_| {
                    let result = HTML_PARSER_DATA.with(|d| d.borrow().clone());
                    HTML_PARSER_DATA.with(|d| d.borrow_mut().clear());
                    Ok(py_str(&result))
                },
            }));

            // getpos() — returns (1, 0)
            dict.insert_str("getpos", PyObjectRef::new(PyObject::BuiltinFunction {
                name: "getpos".to_string(),
                func: |_| Ok(py_tuple(vec![py_int(1), py_int(0)])),
            }));

            Ok(PyObjectRef::new(PyObject::Instance {
                typ: py_str("HTMLParser"),
                dict,
            }))
        },
    });
    d.insert_str("HTMLParser", html_parser);

    d
}

