use crate::object::*;
use std::collections::HashMap;

pub fn create_fnmatch_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! fnmatch_func {
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

    fn fnmatch_match(name: &str, pattern: &str) -> bool {
        // Match through the real translate() output (anchored at the start:
        // the translated pattern ends in `\z` but has no leading anchor, and
        // re.match semantics pin position 0) so `[...]`/`[!...]`/`?` and the
        // other shell constructs behave exactly like CPython. Compiled with
        // fancy_regex (the engine the native `re` module uses) because the
        // plain `regex` crate can't parse translate()'s `(?>...)` atomic
        // groups / `(?s:...)` flag groups.
        let anchored = format!("^{}", fnmatch_translate_str(pattern));
        fancy_regex::Regex::new(&anchored)
            .map(|re| re.is_match(name).unwrap_or(false))
            .unwrap_or(false)
    }

    fn re_escape_char(c: char) -> String {
        // CPython's `re.escape` special-char set: the ASCII punctuation +
        // whitespace below. Note `.` is deliberately NOT escaped (a literal
        // `.` in a shell pattern is a regex any-char — CPython documents
        // "there is no way to quote meta-characters").
        let special = "()[]{}?*+-|^$\\.&~# \t\n\r\u{0b}\u{0c}";
        if special.contains(c) {
            format!("\\{}", c)
        } else {
            c.to_string()
        }
    }

    // Faithful port of CPython's `fnmatch._translate` + `_join_translated_parts`
    // (Lib/fnmatch.py). Returns the EXACT regex source string CPython's
    // `translate()` produces, which its own test suite asserts byte-for-byte
    // (`(?s:...)\z` wrapper, `(?>.*?...)` atomic groups around interior
    // stars, `[^...]` for negated classes, `(?!)` for empty classes, ...).
    fn fnmatch_translate_inner(
        pat: &str,
        star: &str,
        question_mark: &str,
    ) -> (Vec<String>, Vec<usize>) {
        let chars: Vec<char> = pat.chars().collect();
        let n = chars.len();
        let mut res: Vec<String> = Vec::new();
        let mut star_indices: Vec<usize> = Vec::new();
        let mut i = 0usize;
        while i < n {
            let c = chars[i];
            i += 1;
            if c == '*' {
                star_indices.push(res.len());
                res.push(star.to_string());
                while i < n && chars[i] == '*' {
                    i += 1;
                }
            } else if c == '?' {
                res.push(question_mark.to_string());
            } else if c == '[' {
                let mut j = i;
                if j < n && chars[j] == '!' {
                    j += 1;
                }
                if j < n && chars[j] == ']' {
                    j += 1;
                }
                while j < n && chars[j] != ']' {
                    j += 1;
                }
                if j >= n {
                    res.push("\\[".to_string());
                } else {
                    let mut stuff: String = chars[i..j].iter().collect();
                    if !stuff.contains('-') {
                        stuff = stuff.replace('\\', r"\\");
                    } else {
                        let mut chunks: Vec<Vec<char>> = Vec::new();
                        let mut k = if chars[i] == '!' { i + 2 } else { i + 1 };
                        let mut ii = i;
                        loop {
                            let mut found = None;
                            for idx in k..j {
                                if chars[idx] == '-' {
                                    found = Some(idx);
                                    break;
                                }
                            }
                            match found {
                                None => break,
                                Some(kk) => {
                                    chunks.push(chars[ii..kk].to_vec());
                                    ii = kk + 1;
                                    k = kk + 3;
                                }
                            }
                        }
                        let chunk = &chars[ii..j];
                        if !chunk.is_empty() {
                            chunks.push(chunk.to_vec());
                        } else if let Some(last) = chunks.last_mut() {
                            last.push('-');
                        }
                        // Remove empty ranges -- invalid in RE.
                        let mut ck = chunks.len() - 1;
                        while ck > 0 {
                            if chunks[ck - 1].last().unwrap() > &chunks[ck][0] {
                                let mut merged = chunks[ck - 1].clone();
                                merged.pop();
                                merged.extend_from_slice(&chunks[ck][1..]);
                                chunks[ck - 1] = merged;
                                chunks.remove(ck);
                            }
                            ck -= 1;
                        }
                        // Escape backslashes and hyphens for set difference (--).
                        // Hyphens that create ranges shouldn't be escaped.
                        stuff = chunks
                            .iter()
                            .map(|s| {
                                String::from_iter(s)
                                    .replace('\\', r"\\")
                                    .replace('-', r"\-")
                            })
                            .collect::<Vec<_>>()
                            .join("-");
                    }
                    i = j + 1;
                    if stuff.is_empty() {
                        res.push("(?!)".to_string());
                    } else if stuff == "!" {
                        res.push(".".to_string());
                    } else {
                        // Escape set operations (&&, ~~ and ||).
                        stuff = stuff
                            .replace('&', r"\&")
                            .replace('~', r"\~")
                            .replace('|', r"\|");
                        if stuff.starts_with('!') {
                            stuff = format!("^{}", &stuff[1..]);
                        } else if stuff.starts_with('^') || stuff.starts_with('[') {
                            stuff = format!("\\{}", stuff);
                        }
                        res.push(format!("[{}]", stuff));
                    }
                }
            } else {
                res.push(re_escape_char(c));
            }
        }
        (res, star_indices)
    }

    fn fnmatch_join_translated(parts: &[String], star_indices: &[usize]) -> String {
        if star_indices.is_empty() {
            return format!("(?s:{})\\z", parts.concat());
        }
        let mut buffer: Vec<String> = Vec::new();
        let mut iter = star_indices.iter();
        let mut j = *iter.next().unwrap();
        buffer.extend(parts[..j].iter().cloned());
        let mut i2 = j + 1;
        for jj in iter {
            buffer.push("(?>.*?".to_string());
            buffer.extend(parts[i2..*jj].iter().cloned());
            buffer.push(")".to_string());
            i2 = *jj + 1;
        }
        buffer.push(".*".to_string());
        buffer.extend(parts[i2..].iter().cloned());
        format!("(?s:{})\\z", buffer.concat())
    }

    fn fnmatch_translate_str(pat: &str) -> String {
        let (parts, indices) = fnmatch_translate_inner(pat, "*", ".");
        fnmatch_join_translated(&parts, &indices)
    }

    // Real CPython's `_compile_pattern` compiles a bytes pattern to a bytes
    // regex and a str pattern to a str regex; matching a str name against a
    // bytes pattern (or vice versa) raises TypeError from `re.match`. Emulate
    // that check so `fnmatch('test', b'*')` raises instead of silently
    // (lossily) decoding.
    fn fnmatch_type_mismatch(a: &PyObjectRef, b: &PyObjectRef) -> bool {
        let kind = |obj: &PyObjectRef| {
            if matches!(&*obj.borrow(), PyObject::Bytes(_)) {
                1
            } else if matches!(obj, PyObjectRef::SmallStr(_))
                || matches!(&*obj.borrow(), PyObject::Str(_))
            {
                2
            } else {
                0
            }
        };
        let (ka, kb) = (kind(a), kind(b));
        (ka == 1 && kb == 2) || (ka == 2 && kb == 1)
    }

    fnmatch_func!("fnmatch", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("fnmatch() takes exactly 2 arguments"));
        }
        if fnmatch_type_mismatch(&args[0], &args[1]) {
            return Err(PyError::type_error(
                "cannot use a string pattern on a bytes-like object",
            ));
        }
        let name = args[0].str();
        let pattern = args[1].str();
        Ok(py_bool(fnmatch_match(&name, &pattern)))
    });
    // fnmatchcase(name, pattern) — always case-sensitive (unlike `fnmatch`,
    // which normalizes case on case-insensitive filesystems via
    // os.path.normcase). Our `fnmatch_match` never does that normalization
    // to begin with, so this is simply the same matcher under its other
    // real name — was missing entirely (`from fnmatch import fnmatchcase`,
    // real code in CPython's own `unittest.util`).
    fnmatch_func!("fnmatchcase", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error(
                "fnmatchcase() takes exactly 2 arguments",
            ));
        }
        if fnmatch_type_mismatch(&args[0], &args[1]) {
            return Err(PyError::type_error(
                "cannot use a string pattern on a bytes-like object",
            ));
        }
        let name = args[0].str();
        let pattern = args[1].str();
        Ok(py_bool(fnmatch_match(&name, &pattern)))
    });
    fnmatch_func!("translate", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("translate() takes exactly 1 argument"));
        }
        Ok(py_str(&fnmatch_translate_str(&args[0].str())))
    });
    fnmatch_func!("_translate", |args| {
        if args.len() < 3 {
            return Err(PyError::type_error(
                "_translate() takes exactly 3 arguments",
            ));
        }
        let (parts, indices) =
            fnmatch_translate_inner(&args[0].str(), &args[1].str(), &args[2].str());
        Ok(py_tuple(vec![
            py_list(parts.into_iter().map(|s| py_str(&s)).collect()),
            py_list(indices.into_iter().map(|i| py_int(i as i64)).collect()),
        ]))
    });
    fnmatch_func!("_join_translated_parts", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error(
                "_join_translated_parts() takes exactly 2 arguments",
            ));
        }
        let mut parts: Vec<String> = Vec::new();
        if let PyObject::List(items) = &*args[0].borrow() {
            for item in items {
                parts.push(item.str());
            }
        }
        let mut indices: Vec<usize> = Vec::new();
        if let PyObject::List(items) = &*args[1].borrow() {
            for item in items {
                if let Some(i) = item.as_i64() {
                    indices.push(i as usize);
                }
            }
        }
        Ok(py_str(&fnmatch_join_translated(&parts, &indices)))
    });
    fnmatch_func!("filter", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("filter() takes exactly 2 arguments"));
        }
        let pat = args[1].str();
        let mut out = Vec::new();
        if let Ok(it) = builtin_iter(&[args[0].clone()]) {
            loop {
                match builtin_next(&[it.clone()]) {
                    Ok(v) => {
                        if fnmatch_type_mismatch(&v, &args[1]) {
                            return Err(PyError::type_error(
                                "cannot use a string pattern on a bytes-like object",
                            ));
                        }
                        if fnmatch_match(&v.str(), &pat) {
                            out.push(v);
                        }
                    }
                    Err(PyError::StopIteration) => break,
                    Err(e) => return Err(e),
                }
            }
        }
        Ok(py_list(out))
    });
    fnmatch_func!("filterfalse", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error(
                "filterfalse() takes exactly 2 arguments",
            ));
        }
        let pat = args[1].str();
        let mut out = Vec::new();
        if let Ok(it) = builtin_iter(&[args[0].clone()]) {
            loop {
                match builtin_next(&[it.clone()]) {
                    Ok(v) => {
                        if fnmatch_type_mismatch(&v, &args[1]) {
                            return Err(PyError::type_error(
                                "cannot use a string pattern on a bytes-like object",
                            ));
                        }
                        if !fnmatch_match(&v.str(), &pat) {
                            out.push(v);
                        }
                    }
                    Err(PyError::StopIteration) => break,
                    Err(e) => return Err(e),
                }
            }
        }
        Ok(py_list(out))
    });
    d
}
