use crate::object::*;
use std::collections::HashMap;

mod compile;
pub use compile::*;

mod match_obj;
pub use match_obj::*;

/// Wrap a backing dict in a read-only `mappingproxy`-like Instance
/// (CPython's `re.Pattern.groupindex`): reads/iteration work, item
/// assignment raises TypeError (no `__setitem__` in the type dict).
/// Same "last arg is the real key" trick as `types.MappingProxyType`
/// for the two shapes (bound `m[k]`/`len(m)` pass self first, plain
/// `m.get(k)` passes nothing).
pub(crate) fn make_readonly_mapping(inner: PyObjectRef) -> PyObjectRef {
    let mut dict = AttrMap::new();
    let key_arg = |args: &[PyObjectRef]| args.last().cloned();
    let typ = PyObjectRef::new(PyObject::Type {
        name: "mappingproxy".to_string(),
        dict: Box::new(str_map_to_typedict({
            let mut td = HashMap::new();
            for (name, field) in [
                ("keys", "keys"),
                ("values", "values"),
                ("items", "items"),
                ("__len__", "len"),
                ("__iter__", "keys"),
            ] {
                let inner = inner.clone();
                let field = field.to_string();
                td.insert_str(
                    name,
                    PyObjectRef::new(PyObject::Closure(std::rc::Rc::new(
                        move |_args: &[PyObjectRef]| {
                            if let PyObject::Dict(d) = &*inner.borrow() {
                                match field.as_str() {
                                    "keys" => Ok(py_list(d.keys().iter().cloned().collect())),
                                    "values" => Ok(py_list(d.values().iter().cloned().collect())),
                                    "items" => Ok(py_list(
                                        d.items()
                                            .into_iter()
                                            .map(|(k, v)| py_tuple(vec![k, v]))
                                            .collect(),
                                    )),
                                    "len" => Ok(py_int(d.len() as i64)),
                                    _ => Err(PyError::runtime_error("unhandled mapping field")),
                                }
                            } else {
                                Err(PyError::type_error("readonly mapping wrapping a non-dict"))
                            }
                        },
                    ))),
                );
            }
            for (name, field) in [
                ("get", "get"),
                ("__getitem__", "getitem"),
                ("__contains__", "contains"),
            ] {
                let inner = inner.clone();
                let field = field.to_string();
                td.insert_str(
                    name,
                    PyObjectRef::new(PyObject::Closure(std::rc::Rc::new(
                        move |args: &[PyObjectRef]| {
                            let k = key_arg(args)
                                .ok_or_else(|| PyError::type_error("missing key argument"))?;
                            if let PyObject::Dict(d) = &*inner.borrow() {
                                match field.as_str() {
                                    "contains" => {
                                        Ok(py_bool(d.contains(&k).unwrap_or(false)))
                                    }
                                    "get" => match d.get(&k).ok().flatten() {
                                        Some(v) => Ok(v),
                                        None => Ok(args
                                            .get(1)
                                            .cloned()
                                            .unwrap_or_else(|| PyObjectRef::new(PyObject::None))),
                                    },
                                    "getitem" => match d.get(&k).ok().flatten() {
                                        Some(v) => Ok(v),
                                        None => Err(PyError::key_error(k.repr())),
                                    },
                                    _ => Err(PyError::runtime_error("unhandled mapping field")),
                                }
                            } else {
                                Err(PyError::type_error("readonly mapping wrapping a non-dict"))
                            }
                        },
                    ))),
                );
            }
            for (name, message) in [
                ("__setitem__", "mappingproxy object does not support item assignment"),
                ("__delitem__", "mappingproxy object does not support item deletion"),
            ] {
                let message = message.to_string();
                td.insert_str(
                    name,
                    PyObjectRef::new(PyObject::Closure(std::rc::Rc::new(
                        move |_args: &[PyObjectRef]| Err(PyError::type_error(message.clone())),
                    ))),
                );
            }
            td
        })),
        bases: vec![],
        mro: vec![],
    });
    dict.insert(NATIVE_BACKING_KEY.to_string(), inner.clone());
    PyObjectRef::new(PyObject::Instance { typ, dict })
}

/// Decode a regex pattern-or-subject argument: bytes-like objects
/// (bytes/bytearray, their subclasses, memoryview) decode via latin-1
/// (1:1 byte mapping, mirroring CPython's bytes-pattern semantics);
/// returns (text, is_bytes).
/// Whether `arg` is a string/bytes-like value the `re` module accepts as a
/// pattern or subject (str/bytes/bytearray/memoryview/array or a subclass
/// carrying one of those native backings). Used to reject `re.compile(0)` etc.
pub(crate) fn re_is_text_like(arg: &PyObjectRef) -> bool {
    match &*arg.borrow() {
        PyObject::Str(_) | PyObject::Bytes(_) | PyObject::ByteArray(_)
        | PyObject::MemoryView { .. } | PyObject::Array(_) => true,
        PyObject::Instance { .. } => crate::object::native_backing_of(arg).is_some(),
        _ => false,
    }
}

pub(crate) fn re_text_arg(arg: &PyObjectRef) -> (String, bool) {
    match &*arg.borrow() {
        PyObject::Bytes(b) => (b.iter().map(|&x| x as char).collect(), true),
        PyObject::ByteArray(b) => (b.iter().map(|&x| x as char).collect(), true),
        // `array.array` exposes the buffer protocol, so a bytes pattern
        // matches against it (CPython: `re.compile(b"").match(array)` works).
        PyObject::Array(arr) => {
            let mut bytes: Vec<u8> = Vec::new();
            for &v in &arr.data {
                bytes.extend(crate::object::array_elem_to_bytes(arr.typecode, v));
            }
            (bytes.iter().map(|&x| x as char).collect(), true)
        }
        PyObject::MemoryView { .. } => match crate::object::mv_tobytes(arg) {
            Ok(bytes) => (bytes.iter().map(|&x| x as char).collect(), true),
            Err(_) => (arg.str(), false),
        },
        PyObject::Instance { .. } => {
            // bytes/bytearray subclasses carry the native backing.
            match crate::object::native_backing_of(arg) {
                Some(backing) => {
                    let (s, b) = re_text_arg(&backing);
                    // Only accept genuine bytes backings, not e.g. ints.
                    if b {
                        (s, true)
                    } else {
                        (arg.str(), false)
                    }
                }
                None => (arg.str(), false),
            }
        }
        _ => (arg.str(), false),
    }
}

/// Encode text back into a bytes object (latin-1 inverse of `re_text_arg`).
pub(crate) fn re_bytes_obj(s: &str) -> PyObjectRef {
    PyObjectRef::new(PyObject::Bytes(s.chars().map(|c| c as u8).collect()))
}

/// Whether `repl` is a string template (str/bytes/bytearray, or a
/// subclass carrying one of those native backings) rather than a
/// callable replacement function.
pub(crate) fn re_is_template_repl(repl: &PyObjectRef) -> bool {
    match &*repl.borrow() {
        PyObject::Str(_) | PyObject::Bytes(_) | PyObject::ByteArray(_) | PyObject::MemoryView { .. } => {
            true
        }
        PyObject::Instance { .. } => match crate::object::native_backing_of(repl) {
            Some(backing) => matches!(
                &*backing.borrow(),
                PyObject::Str(_) | PyObject::Bytes(_) | PyObject::ByteArray(_)
            ),
            None => false,
        },
        _ => false,
    }
}

/// Validate flag combinations against the pattern flavor (mirrors
/// CPython's `fix_flags`): LOCALE is bytes-only, UNICODE is str-only,
/// and ASCII is incompatible with both UNICODE and LOCALE.
pub(crate) fn re_check_flags(flags: i32, is_bytes: bool) -> PyResult<()> {
    // LOCALE = 4, UNICODE = 32, ASCII = 256.
    if !is_bytes {
        if flags & 4 != 0 {
            return Err(PyError::value_error(
                "cannot use LOCALE flag with a str pattern",
            ));
        }
        if flags & 256 != 0 && flags & 32 != 0 {
            return Err(PyError::value_error(
                "ASCII and UNICODE flags are incompatible",
            ));
        }
    } else {
        if flags & 32 != 0 {
            return Err(PyError::value_error(
                "cannot use UNICODE flag with a bytes pattern",
            ));
        }
        if flags & 256 != 0 && flags & 4 != 0 {
            return Err(PyError::value_error(
                "ASCII and LOCALE flags are incompatible",
            ));
        }
    }
    Ok(())
}

/// Reject mixed str/bytes pattern+subject (mirrors CPython); returns
/// the shared bytes-ness on success.
pub(crate) fn re_check_compat(pat_bytes: bool, subj_bytes: bool) -> PyResult<bool> {
    match (pat_bytes, subj_bytes) {
        (true, false) => Err(PyError::type_error(
            "cannot use a bytes pattern on a string-like object",
        )),
        (false, true) => Err(PyError::type_error(
            "cannot use a string pattern on a bytes-like object",
        )),
        (b, _) => Ok(b),
    }
}

/// Extract a regex and its pattern string from either a raw string or a
/// pre-compiled `re.Pattern` (`PyObject::CompiledRegex`). Also returns
/// whether the pattern is bytes-flavored.
fn extract_regex(arg: &PyObjectRef) -> PyResult<(fancy_regex::Regex, String, bool)> {
    let (re, pattern, is_bytes, _flags) = extract_regex_flags(arg, 0)?;
    Ok((re, pattern, is_bytes))
}

/// Like `extract_regex`, honoring caller-supplied flags for raw
/// patterns (mirrors CPython: `re.search(p, s, re.ASCII)`). Also returns
/// the EFFECTIVE flags (stored flags for a compiled pattern, or the
/// validated/merged flags for a raw one) so callers like `fullmatch` can
/// recompile an anchored variant with identical semantics.
fn extract_regex_flags(
    arg: &PyObjectRef,
    flags: i32,
) -> PyResult<(fancy_regex::Regex, String, bool, i32)> {
    match &*arg.borrow() {
        PyObject::CompiledRegex {
            regex,
            pattern,
            flags: stored_flags,
            is_bytes,
            ..
        } => Ok(((**regex).clone(), pattern.clone(), *is_bytes, *stored_flags)),
        _ => {
            let (pattern, is_bytes) = re_text_arg(arg);
            re_check_flags(flags, is_bytes)?;
            // Validate inline flag groups with CPython's exact errors
            // and rewrite `a`/`L` groups the engine rejects. Keep the
            // original `flags` for validation: only the engine-facing
            // flags below gain the implicit ASCII bit for bytes patterns.
            let (pattern, ascii_scope, global_bits) =
                match compile::preprocess_inline_flags(&pattern, is_bytes, flags) {
                    Ok((p, a, g)) => (p, a, g),
                    Err((msg, pos)) => {
                        return Err(re_pattern_error(msg, Some(pattern), Some(pos as i64)));
                    }
                };
            // Inline globals participate in combination validation
            // (mirrors CPython: `re.compile(b'(?L)', re.ASCII)` is a
            // ValueError).
            re_check_flags(flags | global_bits, is_bytes)?;
            // `(?a)` scope forces ASCII; bytes patterns use ASCII word
            // semantics for the engine (validation above already ran on
            // the original flags, so LOCALE+ASCII is never re-checked).
            let mut flags = flags | global_bits;
            if ascii_scope || is_bytes {
                flags |= 256;
            }
            match compile_python_regex_flags(&pattern, flags, is_bytes) {
                Ok(re) => Ok((re, pattern.clone(), is_bytes, flags)),
                Err(e) => Err(fancy_error_to_pattern(&e, &pattern)),
            }
        }
    }
}


/// Convert a byte index (as reported by the regex engine) into a
/// char index (as CPython reports positions). ASCII-fast: byte and
/// char indices coincide below the first multibyte char.
pub(crate) fn byte_to_char_idx(s: &str, byte_idx: usize) -> usize {
    let byte_idx = byte_idx.min(s.len());
    if s.is_ascii() {
        return byte_idx;
    }
    s[..byte_idx].chars().count()
}

/// Resolve the trailing-`count`/`flags` (or `maxsplit`) arguments for a
/// module-level `re` function, mirroring CPython 3.14:
/// - positional `count`/`maxsplit` (the 4th positional arg for
///   `sub`/`subn`/`split`) raises a `DeprecationWarning` ("'count' is
///   passed as positional argument") — real script_helper callers pass
///   these by position.
/// - a value for the same slot via both position AND kwarg raises
///   `TypeError: ... got multiple values for argument 'count'`.
/// - exceeding the positional bound raises the `takes from N to M
///   positional arguments but K were given` TypeError.
/// Returns `(count_or_maxsplit, flags)`.
pub(crate) fn re_resolve_count_flags(
    args: &[PyObjectRef],
    fname: &str,
    count_kw: &str,
    min_pos: usize,
    max_pos: usize,
) -> PyResult<(i64, i64)> {
    let has_kwargs = match args.last() {
        Some(a) => matches!(&*a.borrow(), PyObject::Dict(_)),
        None => false,
    };
    let kw = if has_kwargs {
        if let PyObject::Dict(d) = &*args.last().unwrap().borrow() {
            Some(d.clone())
        } else {
            None
        }
    } else {
        None
    };
    let npos = args.len() - usize::from(has_kwargs);
    if npos > max_pos {
        return Err(PyError::type_error(format!(
            "{}() takes from {} to {} positional arguments but {} were given",
            fname, min_pos, max_pos, npos
        )));
    }
    let kw_get = |k: &str| -> Option<PyObjectRef> {
        kw.as_ref().and_then(|d| d.get(&py_str(k)).ok().flatten().map(|v| v.clone()))
    };
    if npos >= min_pos + 1 && kw_get(count_kw).is_some() {
        return Err(PyError::type_error(format!(
            "{}() got multiple values for argument '{}'",
            fname, count_kw
        )));
    }
    if npos >= min_pos + 2 && kw_get("flags").is_some() {
        return Err(PyError::type_error(format!(
            "{}() got multiple values for argument 'flags'",
            fname
        )));
    }
    if npos >= min_pos + 1 && kw.is_none() {
        crate::modules::warnings_emit(
            &format!("'{}' is passed as positional argument", count_kw),
            "DeprecationWarning",
        );
    }
    let count = if npos >= min_pos + 1 {
        args[min_pos].as_i64().unwrap_or(0)
    } else {
        kw_get(count_kw)
            .and_then(|v| v.as_i64())
            .unwrap_or(0)
    };
    let flags = if npos >= min_pos + 2 {
        args[min_pos + 1].as_i64().unwrap_or(0)
    } else {
        kw_get("flags")
            .and_then(|v| v.as_i64())
            .unwrap_or(0)
    };
    Ok((count, flags))
}

/// Match sequence for `split`, mirroring CPython's own matcher order:

/// Build one `findall` result for a match: full-match string when the
/// pattern has no groups, the single group (or `None`) with one group,
/// and a tuple of groups otherwise (mirrors CPython `re.findall`).
pub(crate) fn findall_groups(
    caps: &fancy_regex::Captures,
    n_groups: usize,
    is_bytes: bool,
) -> PyObjectRef {
    let piece = |m: Option<&fancy_regex::Match>| -> PyObjectRef {
        // CPython's findall reports unparticipated groups as '' (not None).
        match m {
            Some(m) => {
                if is_bytes {
                    re_bytes_obj(m.as_str())
                } else {
                    py_str(m.as_str())
                }
            }
            None => {
                if is_bytes {
                    re_bytes_obj("")
                } else {
                    py_str("")
                }
            }
        }
    };
    if n_groups == 0 {
        return piece(caps.get(0).as_ref());
    }
    if n_groups == 1 {
        return piece(caps.get(1).as_ref());
    }
    let items: Vec<PyObjectRef> = (1..=n_groups).map(|i| piece(caps.get(i).as_ref())).collect();
    py_tuple(items)
}

/// Convert a char index (CPython position argument) into a byte index
/// for the regex engine.
pub(crate) fn char_to_byte_idx(s: &str, char_idx: usize) -> usize {
    if s.is_ascii() {
        return char_idx.min(s.len());
    }
    let mut count = 0;
    for (b, _) in s.char_indices() {
        if count >= char_idx {
            return b;
        }
        count += 1;
    }
    s.len()
}

/// True when the final `args` entry is this project's packed kwargs dict.
pub(crate) fn has_packed_kwargs(args: &[PyObjectRef]) -> bool {
    match args.last() {
        Some(a) => matches!(&*a.borrow(), PyObject::Dict(_)),
        None => false,
    }
}

/// Read the subject string for Pattern methods: positional `args[0]`, or
/// the `string=` keyword from the packed trailing kwargs dict.
pub(crate) fn re_method_string(args: &[PyObjectRef]) -> (String, bool) {
    let has_kw = has_packed_kwargs(args);
    let npos = if has_kw { args.len() - 1 } else { args.len() };
    if npos >= 1 {
        re_text_arg(&args[0])
    } else if let PyObject::Dict(d) = &*args[0].borrow() {
        if let Some(v) = d.get(&py_str("string")).ok().flatten() {
            re_text_arg(&v)
        } else {
            re_text_arg(&args[0])
        }
    } else {
        re_text_arg(&args[0])
    }
}

/// Read `pos`/`endpos` (CPython char indices) for Pattern methods, from
/// positional args or the packed trailing kwargs dict. `full_len` is the
/// subject's char count (used as the default `endpos`). Missing values
/// fall back to `pos`/`endpos` defaults of CPython.
pub(crate) fn re_pos_endpos(args: &[PyObjectRef], full_len: usize) -> (usize, usize) {
    let has_kw = has_packed_kwargs(args);
    let npos = if has_kw { args.len() - 1 } else { args.len() };
    let kwval = |key: &str| -> Option<i64> {
        if !has_kw {
            return None;
        }
        if let PyObject::Dict(d) = &*args.last().unwrap().borrow() {
            if let Some(v) = d.get(&py_str(key)).ok().flatten() {
                return v.as_i64();
            }
        }
        None
    };
    let pos_c = if npos > 1 {
        args[1].as_i64().or_else(|| kwval("pos"))
    } else {
        kwval("pos")
    }
    .unwrap_or(0)
    .max(0) as usize;
    let endpos_c = if npos > 2 {
        args[2].as_i64().or_else(|| kwval("endpos"))
    } else {
        kwval("endpos")
    }
    .unwrap_or(full_len as i64)
    .max(0) as usize;
    (pos_c, endpos_c)
}

/// Left-to-right match scan over `s[pos_byte..endpos_byte]` in CPython
/// order: adjacent empty matches are kept (probing the end of every
/// non-empty match too) and empty matches advance one char. Match
/// positions are relative to the returned slice (add `pos_byte` to get
/// absolute byte offsets). Mirrors `split_match_spans` but with bounds.
pub(crate) fn scan_captures<'t>(
    re: &fancy_regex::Regex,
    s: &'t str,
    pos_byte: usize,
    endpos_byte: usize,
) -> Vec<fancy_regex::Captures<'t>> {
    let start = pos_byte.min(s.len());
    let end = endpos_byte.min(s.len()).max(start);
    let sub: &'t str = &s[start..end];
    let mut cur = 0usize;
    let mut out = Vec::new();
    while cur <= sub.len() {
        let caps = match re.captures_from_pos(sub, cur) {
            Ok(Some(c)) => c,
            _ => break,
        };
        let m = caps.get(0).unwrap();
        let (ms, me) = (m.start(), m.end());
        out.push(caps);
        if ms == me {
            if me >= sub.len() {
                break;
            }
            cur = me + sub[me..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
        } else {
            cur = me;
        }
    }
    out
}

/// Match sequence for `split`, mirroring CPython's own matcher order:
/// `fancy_regex`'s iterator skips empty matches adjacent to a previous
/// match end, but CPython yields them (their `None` groups appear in
/// the output, e.g. `re.split('(:)*', ...)`). After each non-empty
/// match ending at `e`, probe `e` for an adjacent empty match.
/// Empty matches advance one char (sre progress rule); iteration stops
/// past the end of the string.
pub(crate) fn split_match_spans<'t>(
    re: &fancy_regex::Regex,
    s: &'t str,
) -> Vec<fancy_regex::Captures<'t>> {
    let mut out = Vec::new();
    let mut pos = 0usize;
    while pos <= s.len() {
        let caps = match re.captures_from_pos(s, pos) {
            Ok(Some(c)) => c,
            _ => break,
        };
        let m = caps.get(0).unwrap();
        let (ms, me) = (m.start(), m.end());
        out.push(caps);
        if ms == me {
            if me >= s.len() {
                break;
            }
            pos = me + s[me..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
        } else {
            // Probe for an adjacent empty match at the end position
            // (`captures_from_pos`, unlike the iterator, does not skip
            // them). A consumed probe position must be advanced past,
            // or the main loop re-finds the same empty match.
            if me < s.len() {
                if let Ok(Some(c2)) = re.captures_from_pos(s, me) {
                    if let Some(m2) = c2.get(0) {
                        if m2.start() == me && m2.end() == me {
                            out.push(c2);
                            pos = me
                                + s[me..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
                            continue;
                        }
                    }
                }
            }
            pos = me;
        }
    }
    out
}

/// Read an optional `flags` argument: positional int at `idx`, or a
/// trailing kwargs dict's `"flags"` entry (this project's calling
/// convention packs kwargs as a trailing dict).
fn re_call_flags(args: &[PyObjectRef], idx: usize) -> i32 {
    let mut flags = 0;
    if let Some(a) = args.get(idx) {
        if !matches!(&*a.borrow(), PyObject::Dict(_)) {
            flags |= a.as_i64().unwrap_or(0) as i32;
        }
    }
    if let Some(last) = args.last() {
        if let PyObject::Dict(d) = &*last.borrow() {
            flags |= d
                .get(&py_str("flags"))
                .ok()
                .flatten()
                .and_then(|v| v.as_i64())
                .unwrap_or(0) as i32;
        }
    }
    flags
}

/// Build a `re.Match` for `fullmatch` semantics: the match must span
/// exactly the char range `[pos, endpos)`. Recompiles `pattern` anchored
/// as `(?:pattern)\z` (with `flags`) so the engine backtracks until one
/// alternative consumes the whole window — the plain
/// "take the leftmost match, then filter" approach wrongly rejected
/// `re.fullmatch(r"a|ab", "ab")` (leftmost `a` never spans "ab").
/// The subject is truncated to `endpos` so `\z` anchors at `endpos`
/// (mirroring CPython, where `$`/`\z` match at `endpos`), while
/// `captures_from_pos(pos)` keeps pre-`pos` context visible for
/// `^`/`\b`/lookbehind.
pub fn fullmatch_make(
    pattern: &str,
    flags: i32,
    string: &str,
    pos_c: usize,
    endpos_c: usize,
    is_bytes: bool,
) -> PyResult<PyObjectRef> {
    let n_chars = string.chars().count();
    let pos_c = pos_c.min(n_chars);
    let endpos_c = endpos_c.min(n_chars);
    let pos_b = char_to_byte_idx(string, pos_c);
    let endpos_b = char_to_byte_idx(string, endpos_c);
    let anchored_src = format!("(?:{})\\z", pattern);
    let anchored = match compile_python_regex_flags(&anchored_src, flags, is_bytes) {
        Ok(r) => r,
        Err(e) => {
            return Err(re_pattern_error(
                map_regex_error(&e.to_string()),
                Some(pattern.to_string()),
                None,
            ))
        }
    };
    // Truncate so `\z` lands on `endpos` (not the real string end).
    let truncated = &string[..endpos_b.min(string.len())];
    let caps = anchored
        .captures_from_pos(truncated, pos_b.min(truncated.len()))
        .unwrap_or(None)
        .filter(|c| c.get(0).map(|m| m.start() == pos_b).unwrap_or(false));
    Ok(make_match_object_detailed(
        &anchored,
        caps,
        string,
        pattern,
        flags,
        pos_c,
        endpos_c,
        is_bytes,
    ))
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
        let (re, pattern, pat_bytes, _flags) = extract_regex_flags(&args[0], re_call_flags(args, 2))?;
        let (string, subj_bytes) = re_text_arg(&args[1]);
        let is_bytes = re_check_compat(pat_bytes, subj_bytes)?;
        let caps = re.captures(&string).unwrap_or(None);
        Ok(make_match_object_detailed(
            &re,
            caps,
            &string,
            &pattern,
            0,
            0,
            string.chars().count(),
            is_bytes,
        ))
    });

    re_func!("match", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("match() takes at least 2 arguments"));
        }
        let (re, pattern, pat_bytes, _flags) = extract_regex_flags(&args[0], re_call_flags(args, 2))?;
        let (string, subj_bytes) = re_text_arg(&args[1]);
        let is_bytes = re_check_compat(pat_bytes, subj_bytes)?;
        let caps = re.captures(&string).unwrap_or(None);
        let result = match caps {
            Some(c) if c.get(0).map(|m| m.start()) == Some(0) => Some(c),
            _ => None,
        };
        Ok(make_match_object_detailed(
            &re,
            result,
            &string,
            &pattern,
            0,
            0,
            string.chars().count(),
            is_bytes,
        ))
    });

    re_func!("fullmatch", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error(
                "fullmatch() takes at least 2 arguments",
            ));
        }
        // `extract_regex_flags` validates flags/inline groups and yields the
        // effective flags; the returned non-anchored regex is unused —
        // `fullmatch_make` recompiles anchored so the engine backtracks to
        // span the whole string (leftmost-match+filter rejects `a|ab`).
        let (_re, pattern, pat_bytes, flags) =
            extract_regex_flags(&args[0], re_call_flags(args, 2))?;
        let (string, subj_bytes) = re_text_arg(&args[1]);
        let is_bytes = re_check_compat(pat_bytes, subj_bytes)?;
        fullmatch_make(
            &pattern,
            flags,
            &string,
            0,
            string.chars().count(),
            is_bytes,
        )
    });

    re_func!("findall", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("findall() takes at least 2 arguments"));
        }
        let (re, _pattern, pat_bytes, _flags) = extract_regex_flags(&args[0], re_call_flags(args, 2))?;
        let (string, subj_bytes) = re_text_arg(&args[1]);
        let is_bytes = re_check_compat(pat_bytes, subj_bytes)?;
        let groups = re.captures_len().saturating_sub(1);
        let caps_list = scan_captures(&re, &string, 0, string.len());
        let results: Vec<PyObjectRef> = caps_list
            .iter()
            .map(|caps| findall_groups(caps, groups, is_bytes))
            .collect();
        Ok(py_list(results))
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
        let (re, pattern, pat_bytes, _flags) = extract_regex_flags(&args[0], re_call_flags(args, 4))?;
        let is_callable_repl = !re_is_template_repl(&args[1]);
        let (repl_text, repl_bytes) = re_text_arg(&args[1]);
        let (string, subj_bytes) = re_text_arg(&args[2]);
        let is_bytes = re_check_compat(pat_bytes, subj_bytes)?;
        if !is_callable_repl && repl_bytes != subj_bytes {
            return Err(PyError::type_error(
                "cannot use a string replacement on a bytes-like object",
            ));
        }
        let repl_template = if is_callable_repl {
            String::new()
        } else {
            translate_python_replacement(&repl_text)?
        };
        // `count`/`flags` may arrive positionally (args[3]) OR as a trailing
        // kwargs dict (`re.sub(p, r, s, count=1)` — a real, common call
        // shape; this project's calling convention appends a `{"count":
        // ..., "flags": ...}` dict as the final positional arg for keyword
        // calls, same as `sorted`'s own `key=`/`reverse=` handling).
        let (count, _flags) = re_resolve_count_flags(args, "sub", "count", 3, 5)?;
        if !is_callable_repl {
            if let Err(e) = validate_g_template(&repl_text, &re, is_bytes) {
                return Err(e);
            }
            let repl_str = repl_text.clone();
            for cap in re.capture_names().filter_map(|n| n) {
                let _ = cap;
            }
            let mut search_idx = 0;
            let repl_chars: Vec<char> = repl_str.chars().collect();
            while search_idx < repl_chars.len() {
                if repl_chars[search_idx] == '\\'
                    && search_idx + 2 < repl_chars.len()
                    && repl_chars[search_idx + 1] == 'g'
                    && repl_chars[search_idx + 2] == '<'
                {
                    let mut j = search_idx + 3;
                    while j < repl_chars.len() && repl_chars[j] != '>' {
                        j += 1;
                    }
                    if j < repl_chars.len() {
                        let name: String = repl_chars[search_idx + 3..j].iter().collect();
                        if !name.is_empty()
                            && !name.chars().next().unwrap().is_ascii_digit()
                            && !name.chars().all(|c| c.is_ascii_digit())
                        {
                            let found = re.capture_names().any(|n| n == Some(name.as_str()));
                            if !found {
                                return Err(PyError::IndexError(format!(
                                    "unknown group name '{}'",
                                    name
                                )));
                            }
                        }
                    }
                }
                search_idx += 1;
            }
        }
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
                let match_obj = make_match_object_detailed(
                    &re,
                    Some(caps),
                    &string,
                    &pattern,
                    0,
                    m_start,
                    string.len(),
                    is_bytes,
                );
                let replaced = call_bound_method(args[1].clone(), match_obj, vec![])?;
                // A bytes-returning callable decodes via latin-1 so the
                // substitution stays in the subject's domain.
                if is_bytes {
                    let (rs, _) = re_text_arg(&replaced);
                    result.push_str(&rs);
                } else {
                    result.push_str(&replaced.str());
                }
            } else {
                let mut expanded = String::new();
                caps.expand(&repl_template, &mut expanded);
                result.push_str(&expanded);
            }
            last_end = m_end;
            n += 1;
        }
        result.push_str(&string[last_end..]);
        if is_bytes {
            Ok(re_bytes_obj(&result))
        } else {
            Ok(py_str(&result))
        }
    });

    re_func!("split", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("split() takes at least 2 arguments"));
        }
        let (maxsplit_i, flags_i) = re_resolve_count_flags(args, "split", "maxsplit", 2, 4)?;
        let (string, maxsplit, flags) = (re_text_arg(&args[1]), maxsplit_i as usize, flags_i as i32);
        let ((string, subj_bytes), maxsplit, flags) = (string, maxsplit, flags);
        let (re, pat_is_bytes): (fancy_regex::Regex, bool) = match &*args[0].borrow() {
            PyObject::CompiledRegex {
                regex, is_bytes, ..
            } => ((**regex).clone(), *is_bytes),
            _ => {
                let (pattern, pb) = re_text_arg(&args[0]);
                let (pattern, ascii_scope, global_bits) =
                    match compile::preprocess_inline_flags(&pattern, pb, flags) {
                        Ok((p, a, g)) => (p, a, g),
                        Err((msg, pos)) => {
                            return Err(re_pattern_error(
                                msg,
                                Some(pattern),
                                Some(pos as i64),
                            ));
                        }
                    };
                re_check_flags(flags | global_bits, pb)?;
                // Bytes patterns use ASCII word semantics (see `compile`);
                // validation above ran on the original flags.
                let mut flags = flags | global_bits;
                if ascii_scope || pb {
                    flags |= 256;
                }
                match compile_python_regex_flags(&pattern, flags, pb) {
                    Ok(r) => (r, pb),
                    Err(e) => {
                        return Err(fancy_error_to_pattern(&e, &pattern));
                    }
                }
            }
        };
        let is_bytes = re_check_compat(pat_is_bytes, subj_bytes)?;
        // Pieces and captured groups come back in the subject domain.
        let piece = |s: &str| -> PyObjectRef {
            if is_bytes {
                re_bytes_obj(s)
            } else {
                py_str(s)
            }
        };
        // Python's re.split includes captured groups; manual loop over captures
        // in CPython match order (see `split_match_spans`: adjacent
        // empties included).
        let mut result: Vec<PyObjectRef> = Vec::new();
        let mut last_end = 0usize;
        let mut n = 0usize;
        for caps in split_match_spans(&re, &string) {
            if maxsplit != 0 && n >= maxsplit {
                break;
            }
            let m = caps.get(0).unwrap();
            let (s, e) = (m.start(), m.end());
            // Every yielded match (including empty and end-of-string
            // ones) contributes its piece — mirroring CPython, whose
            // own matcher yields them (verified: `re.split('', 'ab')`
            // is `['', 'a', 'b', '']`, `re.split('x*', 'axb')` keeps
            // the adjacent empties). The iterator is finite, so no
            // progress guard is needed here.
            result.push(piece(&string[last_end..s]));
            for i in 1..caps.len() {
                if let Some(g) = caps.get(i) {
                    result.push(piece(g.as_str()));
                } else {
                    result.push(py_none());
                }
            }
            last_end = e;
            n += 1;
        }
        result.push(piece(&string[last_end..]));
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
        // CPython: re.compile(0) → TypeError "first argument must be string
        // or compiled pattern" (reject ints/objects, not just stringify them).
        if !re_is_text_like(&args[0]) {
            return Err(PyError::type_error(
                "first argument must be string or compiled pattern",
            ));
        }
        let (pattern, is_bytes) = re_text_arg(&args[0]);
        let flags = if args.len() > 1 {
            args[1].as_i64().unwrap_or(0) as i32
        } else {
            0
        };
        // Bytes patterns use ASCII word semantics (mirrors CPython);
        // the stored `.flags` keeps the caller's value.
        re_check_flags(flags, is_bytes)?;
        let original_pattern = pattern.clone();
        let compile_flags = if is_bytes { flags | 256 } else { flags };
        let (pattern, ascii_scope, global_bits) =
            match compile::preprocess_inline_flags(&original_pattern, is_bytes, flags) {
                Ok((p, a, g)) => (p, a, g),
                Err((msg, pos)) => {
                    return Err(re_pattern_error(msg, Some(original_pattern), Some(pos as i64)));
                }
            };
        re_check_flags(flags | global_bits, is_bytes)?;
        // Inline global flags (e.g. `(?i)`) fold into the stored flags,
        // and the *original* pattern is kept (CPython stores both).
        // `fix_flags` semantics: str patterns imply UNICODE (unless
        // ASCII is set); bytes patterns never carry UNICODE.
        let stored_flags = flags | global_bits;
        let stored_flags = if is_bytes {
            stored_flags & !32
        } else if stored_flags & 256 == 0 {
            stored_flags | 32
        } else {
            stored_flags
        };
        let compile_flags = if ascii_scope {
            compile_flags | 256
        } else {
            compile_flags
        };
        match compile_python_regex_flags(&pattern, compile_flags, is_bytes) {
            Ok(re) => Ok(PyObjectRef::new(PyObject::CompiledRegex {
                regex: Box::new(re),
                pattern: original_pattern,
                flags: stored_flags,
                is_bytes,
            })),
            Err(e) => Err(fancy_error_to_pattern(&e, &pattern)),
        }
    });

    // finditer — returns a list of Match objects (not a lazy iterator, but sufficient for Django)
    re_func!("finditer", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("finditer() takes at least 2 arguments"));
        }
        let (re, pattern, pat_bytes, _flags) = extract_regex_flags(&args[0], re_call_flags(args, 2))?;
        let (string, subj_bytes) = re_text_arg(&args[1]);
        let is_bytes = re_check_compat(pat_bytes, subj_bytes)?;
        let caps_list = scan_captures(&re, &string, 0, string.len());
        let matches: Vec<PyObjectRef> = caps_list
            .into_iter()
            .map(|c| {
                let start = c.get(0).map(|m| m.start()).unwrap_or(0);
                make_match_object_prefixed(
                    &re,
                    Some(c),
                    &string,
                    &pattern,
                    0,
                    start,
                    string.len(),
                    is_bytes,
                    0,
                )
            })
            .collect();
        Ok(py_list(matches))
    });

    re_func!("escape", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("escape() missing required argument"));
        }
        // Bytes input escapes to bytes (mirrors CPython: decode
        // latin-1, escape, re-encode).
        let (s, is_bytes) = re_text_arg(&args[0]);
        // Mirrors CPython's _sre escape: every char/byte whose ordinal is
        // an ASCII non-alphanumeric and not in LITERAL_CHARS gets a '\'
        // prefix; everything >= 0x80 passes through untouched.
        let literals = "!\"%',/:;<=>@_`";
        let mut escaped = String::new();
        for c in s.chars() {
            if (c.is_ascii_alphanumeric() || literals.contains(c))
                && c.is_ascii()
            {
                escaped.push(c);
            } else if c.is_ascii() {
                escaped.push('\\');
                escaped.push(c);
            } else {
                escaped.push(c);
            }
        }
        if is_bytes {
            Ok(re_bytes_obj(&escaped))
        } else {
            Ok(py_str(&escaped))
        }
    });

    // Regex compile flags
    d.insert_str("IGNORECASE", py_int(2));
    d.insert_str("ASCII", py_int(256));
    d.insert_str("DOTALL", py_int(16));
    d.insert_str("MULTILINE", py_int(8));
    d.insert_str("VERBOSE", py_int(64));
    d.insert_str("LOCALE", py_int(4));
    d.insert_str("UNICODE", py_int(32));
    d.insert_str("TEMPLATE", py_int(1));
    d.insert_str("DEBUG", py_int(128));
    d.insert_str("NOFLAG", py_int(0));
    d.insert_str("I", py_int(2));
    d.insert_str("A", py_int(256));
    d.insert_str("S", py_int(16));
    d.insert_str("M", py_int(8));
    d.insert_str("X", py_int(64));
    d.insert_str("L", py_int(4));
    d.insert_str("U", py_int(32));
    d.insert_str("T", py_int(1));

    // RegexFlag — IntFlag-like type with same members as module-level flags
    {
        let mut rf_dict = HashMap::new();
        rf_dict.insert("NOFLAG".to_string(), py_int(0));
        rf_dict.insert("ASCII".to_string(), py_int(256));
        rf_dict.insert("A".to_string(), py_int(256));
        rf_dict.insert("IGNORECASE".to_string(), py_int(2));
        rf_dict.insert("I".to_string(), py_int(2));
        rf_dict.insert("LOCALE".to_string(), py_int(4));
        rf_dict.insert("L".to_string(), py_int(4));
        rf_dict.insert("UNICODE".to_string(), py_int(32));
        rf_dict.insert("U".to_string(), py_int(32));
        rf_dict.insert("MULTILINE".to_string(), py_int(8));
        rf_dict.insert("M".to_string(), py_int(8));
        rf_dict.insert("DOTALL".to_string(), py_int(16));
        rf_dict.insert("S".to_string(), py_int(16));
        rf_dict.insert("VERBOSE".to_string(), py_int(64));
        rf_dict.insert("X".to_string(), py_int(64));
        rf_dict.insert("TEMPLATE".to_string(), py_int(1));
        rf_dict.insert("T".to_string(), py_int(1));
        rf_dict.insert("DEBUG".to_string(), py_int(128));
        let rf_type = PyObjectRef::new(PyObject::Type {
            name: "RegexFlag".to_string(),
            dict: Box::new(str_map_to_typedict(rf_dict)),
            bases: vec![],
            mro: vec![],
        });
        if let PyObject::Type { mro, .. } = &mut *rf_type.borrow_mut() {
            *mro = vec![rf_type.clone()];
        }
        d.insert_str("RegexFlag", rf_type);
    }

    // re.error / re.PatternError — alias, same object
    {
        let pattern_error = PyObjectRef::new(PyObject::BuiltinFunction {
            name: "PatternError".to_string(),
            func: |args| {
                let msg = args.get(0).map(|a| a.str()).unwrap_or_default();
                let pattern = args.get(1).cloned().unwrap_or_else(py_none);
                let pos = args.get(2).cloned().unwrap_or_else(py_none);
                let mut extra = HashMap::new();
                extra.insert("msg".to_string(), py_str(&msg));
                extra.insert("pattern".to_string(), pattern);
                extra.insert("pos".to_string(), pos);
                Ok(PyObjectRef::new(PyObject::Exception {
                    typ: "PatternError".to_string(),
                    args: vec![py_str(&msg)],
                    cause: None,
                    suppress_context: false,
                    context: None,
                    traceback: None,
                    extra: Some(extra),
                }))
            },
        });
        d.insert_str("error", pattern_error.clone());
        d.insert_str("PatternError", pattern_error);
    }

    // purge — clear cache (no-op)
    re_func!("purge", |_| Ok(py_none()));

    // subn — like sub but returns (new_string, count)
    re_func!("subn", |args| {
        if args.len() < 3 {
            return Err(PyError::type_error("subn() takes at least 3 arguments"));
        }
        let (re, pattern, pat_bytes, _flags) = extract_regex_flags(&args[0], re_call_flags(args, 4))?;
        let is_callable_repl = !re_is_template_repl(&args[1]);
        let (repl_text, repl_bytes) = re_text_arg(&args[1]);
        let (string, subj_bytes) = re_text_arg(&args[2]);
        let is_bytes = re_check_compat(pat_bytes, subj_bytes)?;
        if !is_callable_repl && repl_bytes != subj_bytes {
            return Err(PyError::type_error(
                "cannot use a string replacement on a bytes-like object",
            ));
        }
        let repl_template = if is_callable_repl {
            String::new()
        } else {
            translate_python_replacement(&repl_text)?
        };
        let (count, _flags) = re_resolve_count_flags(args, "subn", "count", 3, 5)?;
        if !is_callable_repl {
            if let Err(e) = validate_g_template(&repl_text, &re, is_bytes) {
                return Err(e);
            }
            let repl_str = repl_text.clone();
            for cap in re.capture_names().filter_map(|n| n) {
                let _ = cap;
            }
            let mut search_idx = 0;
            let repl_chars: Vec<char> = repl_str.chars().collect();
            while search_idx < repl_chars.len() {
                if repl_chars[search_idx] == '\\'
                    && search_idx + 2 < repl_chars.len()
                    && repl_chars[search_idx + 1] == 'g'
                    && repl_chars[search_idx + 2] == '<'
                {
                    let mut j = search_idx + 3;
                    while j < repl_chars.len() && repl_chars[j] != '>' {
                        j += 1;
                    }
                    if j < repl_chars.len() {
                        let name: String = repl_chars[search_idx + 3..j].iter().collect();
                        if !name.is_empty()
                            && !name.chars().next().unwrap().is_ascii_digit()
                            && !name.chars().all(|c| c.is_ascii_digit())
                        {
                            let found = re.capture_names().any(|n| n == Some(name.as_str()));
                            if !found {
                                return Err(PyError::IndexError(format!(
                                    "unknown group name '{}'",
                                    name
                                )));
                            }
                        }
                    }
                }
                search_idx += 1;
            }
        }
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
                let match_obj = make_match_object_detailed(
                    &re,
                    Some(caps),
                    &string,
                    &pattern,
                    0,
                    m_start,
                    string.len(),
                    is_bytes,
                );
                let replaced = call_bound_method(args[1].clone(), match_obj, vec![])?;
                // A bytes-returning callable decodes via latin-1 so the
                // substitution stays in the subject's domain.
                if is_bytes {
                    let (rs, _) = re_text_arg(&replaced);
                    result.push_str(&rs);
                } else {
                    result.push_str(&replaced.str());
                }
            } else {
                let mut expanded = String::new();
                caps.expand(&repl_template, &mut expanded);
                result.push_str(&expanded);
            }
            last_end = m_end;
            n += 1;
        }
        result.push_str(&string[last_end..]);
        if is_bytes {
            Ok(py_tuple(vec![re_bytes_obj(&result), py_int(n)]))
        } else {
            Ok(py_tuple(vec![py_str(&result), py_int(n)]))
        }
    });

    // Scanner — tokenizer helper
    {
        let mut scanner_dict = HashMap::new();
        scanner_dict.insert(
            "__init__".to_string(),
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "__init__".to_string(),
                func: |args| {
                    if args.len() < 2 {
                        return Err(PyError::type_error("Scanner.__init__ missing lexicon"));
                    }
                    let self_obj = &args[0];
                    let lexicon = args[1].clone();
                    let flags = args.get(2).and_then(|a| a.as_i64()).unwrap_or(0) as i32;
                    let items: Vec<PyObjectRef> =
                        if let PyObject::List(v) | PyObject::Tuple(v) = &*lexicon.borrow() {
                            v.clone()
                        } else {
                            return Err(PyError::type_error("lexicon must be list/tuple"));
                        };
                    let mut compiled_list: Vec<PyObjectRef> = Vec::new();
                    for item in &items {
                        let pair: Vec<PyObjectRef> =
                            if let PyObject::Tuple(v) | PyObject::List(v) = &*item.borrow() {
                                v.clone()
                            } else {
                                continue;
                            };
                        if pair.len() < 2 {
                            continue;
                        }
                        let pat = pair[0].str();
                        let (pat, _) =
                            match compile::preprocess_inline_flags(&pat, false, flags) {
                                Ok((p, _, _)) => (p, false),
                                Err((msg, pos)) => {
                                    return Err(re_pattern_error(
                                        msg,
                                        Some(pat),
                                        Some(pos as i64),
                                    ));
                                }
                            };
                        match compile_python_regex_flags(&pat, flags, false) {
                            Ok(r) => {
                                compiled_list.push(PyObjectRef::new(PyObject::CompiledRegex {
                                    regex: Box::new(r),
                                    pattern: pat,
                                    flags,
                                    is_bytes: false,
                                }));
                            }
                            Err(e) => {
                                return Err(fancy_error_to_pattern(&e, &pat));
                            }
                        }
                    }
                    let dummy_re = fancy_regex::Regex::new("").unwrap();
                    let dummy = PyObjectRef::new(PyObject::CompiledRegex {
                        regex: Box::new(dummy_re),
                        pattern: "".to_string(),
                        flags: 0,
                        is_bytes: false,
                    });
                    if let PyObject::Instance { dict, .. } = &mut *self_obj.borrow_mut() {
                        dict.insert_str("lexicon", lexicon.clone());
                        dict.insert_str("_compiled", py_list(compiled_list));
                        dict.insert_str("scanner", dummy);
                        dict.insert_str("match", py_none());
                    }
                    Ok(py_none())
                },
            }),
        );
        scanner_dict.insert(
            "scan".to_string(),
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "scan".to_string(),
                func: |args| {
                    if args.is_empty() {
                        return Err(PyError::type_error("scan() missing self"));
                    }
                    let self_obj = &args[0];
                    let string = args.get(1).map(|a| a.str()).unwrap_or_default();
                    let lexicon = self_obj
                        .borrow()
                        .get_attribute("lexicon")
                        .unwrap_or_else(|_| py_list(vec![]));
                    let compiled = self_obj
                        .borrow()
                        .get_attribute("_compiled")
                        .unwrap_or_else(|_| py_list(vec![]));
                    let lex_items: Vec<PyObjectRef> =
                        if let PyObject::List(v) | PyObject::Tuple(v) = &*lexicon.borrow() {
                            v.clone()
                        } else {
                            vec![]
                        };
                    let comp_items: Vec<PyObjectRef> =
                        if let PyObject::List(v) = &*compiled.borrow() {
                            v.clone()
                        } else {
                            vec![]
                        };
                    let mut result: Vec<PyObjectRef> = Vec::new();
                    let mut i: usize = 0;
                    let bytes_len = string.len();
                    // To avoid infinite loops, track if progress is made
                    while i < bytes_len {
                        let mut matched = false;
                        for (idx, comp) in comp_items.iter().enumerate() {
                            let re = if let PyObject::CompiledRegex { regex, .. } = &*comp.borrow()
                            {
                                (**regex).clone()
                            } else {
                                continue;
                            };
                            let caps_opt = re.captures_from_pos(&string, i).unwrap_or(None);
                            if let Some(caps) = caps_opt {
                                if let Some(m) = caps.get(0) {
                                    if m.start() != i {
                                        continue;
                                    }
                                    let j = m.end();
                                    if i == j {
                                        continue;
                                    }
                                    let action = lex_items
                                        .get(idx)
                                        .and_then(|item| {
                                            if let PyObject::Tuple(v) | PyObject::List(v) =
                                                &*item.borrow()
                                            {
                                                v.get(1).cloned()
                                            } else {
                                                None
                                            }
                                        })
                                        .unwrap_or_else(py_none);
                                    let is_none = matches!(&*action.borrow(), PyObject::None);
                                    let mut use_action = true;
                                    let mut final_val: Option<PyObjectRef> = None;
                                    if is_none {
                                        use_action = false;
                                    } else if !matches!(&*action.borrow(), PyObject::None) {
                                        // Check if callable (Function, BuiltinFunction, Closure, etc.)
                                        let is_callable = {
                                            let b = action.borrow();
                                            matches!(
                                                &*b,
                                                PyObject::Function(_)
                                                    | PyObject::BuiltinFunction { .. }
                                                    | PyObject::Closure(_)
                                                    | PyObject::Instance { .. }
                                                    | PyObject::BuiltinMethod { .. }
                                            )
                                        };
                                        if is_callable {
                                            let token_str = py_str(m.as_str());
                                            // set self.match for the callable to inspect
                                            let caps_clone =
                                                re.captures_from_pos(&string, i).unwrap_or(None);
                                            let match_obj = make_match_object_detailed(
                                                &re,
                                                caps_clone,
                                                &string,
                                                "",
                                                0,
                                                i,
                                                string.len(),
                                                false,
                                            );
                                            if let PyObject::Instance { dict, .. } =
                                                &mut *self_obj.borrow_mut()
                                            {
                                                dict.insert_str("match", match_obj.clone());
                                            }
                                            // CPython calls action(scanner, token)
                                            match call_bound_method(
                                                action.clone(),
                                                self_obj.clone(),
                                                vec![token_str.clone()],
                                            ) {
                                                Ok(v) => {
                                                    if matches!(&*v.borrow(), PyObject::None) {
                                                        use_action = false;
                                                    } else {
                                                        final_val = Some(v);
                                                    }
                                                }
                                                Err(_) => {
                                                    // Fallback: action(token) single-arg form
                                                    match call_bound_method(
                                                        action.clone(),
                                                        token_str.clone(),
                                                        vec![],
                                                    ) {
                                                        Ok(v2) => {
                                                            if matches!(
                                                                &*v2.borrow(),
                                                                PyObject::None
                                                            ) {
                                                                use_action = false;
                                                            } else {
                                                                final_val = Some(v2);
                                                            }
                                                        }
                                                        Err(e) => return Err(e),
                                                    }
                                                }
                                            }
                                        } else {
                                            final_val = Some(action.clone());
                                        }
                                    }
                                    if use_action {
                                        if let Some(v) = final_val {
                                            result.push(v);
                                        }
                                    }
                                    i = j;
                                    matched = true;
                                    break;
                                }
                            }
                        }
                        if !matched {
                            break;
                        }
                    }
                    let rest = py_str(&string[i..]);
                    Ok(py_tuple(vec![py_list(result), rest]))
                },
            }),
        );
        let scanner_type = PyObjectRef::new(PyObject::Type {
            name: "Scanner".to_string(),
            dict: Box::new(str_map_to_typedict(scanner_dict)),
            bases: vec![],
            mro: vec![],
        });
        if let PyObject::Type { mro, .. } = &mut *scanner_type.borrow_mut() {
            *mro = vec![scanner_type.clone()];
        }
        d.insert_str("Scanner", scanner_type);
    }

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
        dict: {
            let mut m: HashMap<String, PyObjectRef> = HashMap::new();
            m.insert("__module__".to_string(), py_str("re"));
            Box::new(str_map_to_typedict(m))
        },
        bases: vec![],
        mro: vec![],
    });
    d.insert_str("Match", re_match_type);

    d.insert_str("__version__", py_str("2.2.1"));
    d.insert_str(
        "__all__",
        py_list(vec![
            py_str("match"),
            py_str("fullmatch"),
            py_str("search"),
            py_str("sub"),
            py_str("subn"),
            py_str("split"),
            py_str("findall"),
            py_str("finditer"),
            py_str("compile"),
            py_str("purge"),
            py_str("escape"),
            py_str("error"),
            py_str("Pattern"),
            py_str("Match"),
            py_str("A"),
            py_str("I"),
            py_str("L"),
            py_str("M"),
            py_str("S"),
            py_str("X"),
            py_str("U"),
            py_str("ASCII"),
            py_str("IGNORECASE"),
            py_str("LOCALE"),
            py_str("MULTILINE"),
            py_str("DOTALL"),
            py_str("VERBOSE"),
            py_str("UNICODE"),
            py_str("NOFLAG"),
            py_str("RegexFlag"),
            py_str("PatternError"),
        ]),
    );

    // `re._compiler` — attribute-accessible submodule mirroring CPython's
    // `from . import _compiler` in `re/__init__.py` (so `re._compiler.X`
    // works after just `import re`). Provides the internals `test_re`'s
    // `test_signedness` (MAXREPEAT/MAXGROUPS >= 0) and `test_overlap_table`
    // (`_generate_overlap_table`) assert on. Built natively here (not via
    // a file import) so attribute access needs no import-timing wiring.
    let mut compiler_dict = HashMap::new();
    // Match CPython's `_sre` constants: MAXREPEAT = ~0 (u32), MAXGROUPS =
    // INT32_MAX/2 (see `Modules/_sre/sre.h`).
    compiler_dict.insert_str("MAXREPEAT", py_int(2147483647));
    compiler_dict.insert_str("MAXGROUPS", py_int(1073741823));
    compiler_dict.insert_str(
        "_generate_overlap_table",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "_generate_overlap_table".to_string(),
            // Verbatim port of CPython's KMP self-overlap table generator
            // (`Lib/re/_compiler.py::_generate_overlap_table`), including
            // the `while…else` (break vs. natural-exit) distinction.
            func: |args| {
                if args.is_empty() {
                    return Err(PyError::type_error(
                        "_generate_overlap_table() takes exactly 1 argument",
                    ));
                }
                let prefix: Vec<char> = args[0].str().chars().collect();
                let mut table: Vec<i64> = vec![0; prefix.len()];
                for i in 1..prefix.len() {
                    let mut idx = table[i - 1] as usize;
                    let mut broke = false;
                    while prefix[i] != prefix[idx] {
                        if idx == 0 {
                            table[i] = 0;
                            broke = true;
                            break;
                        }
                        idx = table[idx - 1] as usize;
                    }
                    if !broke {
                        table[i] = idx as i64 + 1;
                    }
                }
                Ok(py_list(table.into_iter().map(py_int).collect()))
            },
        }),
    );
    d.insert_str("_compiler", create_module("re._compiler", compiler_dict));

    d
}
