use crate::object::*;
use std::collections::HashMap as StdHashMap;

use super::dialect::{CsvDialect, CSV_FIELD_LIMIT};

pub(crate) fn parse_csv_lines(lines: Vec<String>, dialect: &CsvDialect) -> PyResult<Vec<Vec<PyObjectRef>>> {
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

pub(crate) fn format_csv_field(field: &str, dialect: &CsvDialect, is_none: bool) -> PyResult<String> {
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

pub(crate) fn format_csv_row(cells: Vec<PyObjectRef>, dialect: &CsvDialect) -> PyResult<String> {
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

pub(crate) fn sniff_guess_quote_and_delimiter(sample: &str, delimiters: Option<&str>) -> (String, bool, Option<char>, bool) {
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

pub(crate) fn sniff_guess_delimiter(sample: &str, delimiters: Option<&str>) -> (Option<char>, bool) {
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

pub(crate) fn csv_has_header(sample: &str) -> PyResult<bool> {
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
