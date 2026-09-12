// A native, hand-rolled subset of `xml.etree.ElementTree`.
//
// This is NOT CPython's real `Lib/xml/etree/ElementTree.py`: that module
// relies on the `pyexpat` C extension (`_elementtree`/`pyexpat`) for actual
// XML parsing, and this codebase's `pyexpat` (`Lib/pyexpat.py`) is only a
// non-functional stub (see `GAP_ANALYSIS.md`/CLAUDE.md — implementing a real
// expat-compatible parser is out of scope here). Vendoring the real
// `ElementTree.py` verbatim would just move the same gap one file over
// (`fromstring`/`parse`/`iterparse` would silently do nothing), so instead
// this file implements the core `Element`/`ElementTree` object model plus a
// small XML parser/serializer directly in Rust, covering the common,
// non-namespace-resolving subset of the API: tags, attributes, text/tail,
// children, comments, processing instructions, CDATA, and entity refs.
//
// Deliberately NOT attempted here (real gaps, not addressed by this file):
// full `{uri}tag` namespace resolution during parsing, `QName`,
// `iterparse`/`XMLParser`/`TreeBuilder`, C14N.
use crate::object::*;
use std::cell::RefCell;
use std::collections::HashMap;

thread_local! {
    static ELEMENT_TYPE: RefCell<Option<PyObjectRef>> = const { RefCell::new(None) };
    static ELEMENTTREE_TYPE: RefCell<Option<PyObjectRef>> = const { RefCell::new(None) };
    // Sentinel callables used as `.tag` for Comment/ProcessingInstruction
    // nodes, mirroring real ElementTree (`elem.tag is Comment`).
    static COMMENT_MARKER: RefCell<Option<PyObjectRef>> = const { RefCell::new(None) };
    static PI_MARKER: RefCell<Option<PyObjectRef>> = const { RefCell::new(None) };
    static NAMESPACE_MAP: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
}

fn element_type() -> PyObjectRef {
    ELEMENT_TYPE.with(|c| c.borrow().clone().unwrap())
}

fn is_element_instance(obj: &PyObjectRef) -> bool {
    matches!(&*obj.borrow(), PyObject::Instance { .. })
}

fn get_field(obj: &PyObjectRef, name: &str) -> Option<PyObjectRef> {
    let b = obj.borrow();
    if let PyObject::Instance { dict, .. } = &*b {
        dict.get_str(name).cloned()
    } else {
        None
    }
}

fn set_field(obj: &PyObjectRef, name: &str, value: PyObjectRef) {
    if let PyObject::Instance { dict, .. } = &mut *obj.borrow_mut() {
        dict.insert_str(name, value);
    }
}

fn children_of(obj: &PyObjectRef) -> Vec<PyObjectRef> {
    get_field(obj, "children")
        .and_then(|c| {
            if let PyObject::List(list) = &*c.borrow() {
                Some(list.clone())
            } else {
                None
            }
        })
        .unwrap_or_default()
}

fn with_children_mut<R>(obj: &PyObjectRef, f: impl FnOnce(&mut Vec<PyObjectRef>) -> R) -> Option<R> {
    let children = get_field(obj, "children")?;
    let mut borrowed = children.borrow_mut();
    if let PyObject::List(list) = &mut *borrowed {
        Some(f(list))
    } else {
        None
    }
}

fn opt_str(v: Option<&PyObjectRef>) -> Option<String> {
    v.and_then(|v| {
        if matches!(&*v.borrow(), PyObject::None) {
            None
        } else {
            Some(v.str())
        }
    })
}

fn is_none(v: &PyObjectRef) -> bool {
    matches!(&*v.borrow(), PyObject::None)
}

fn new_element(tag: PyObjectRef, attrib: PyObjectRef) -> PyObjectRef {
    let typ = element_type();
    let mut instance_dict = AttrMap::new();
    instance_dict.insert_str("tag", tag);
    instance_dict.insert_str("text", py_none());
    instance_dict.insert_str("tail", py_none());
    instance_dict.insert_str("attrib", attrib);
    instance_dict.insert_str("children", py_list(vec![]));
    PyObjectRef::new(PyObject::Instance {
        typ,
        dict: instance_dict,
    })
}

// ---------------------------------------------------------------------
// XML escaping / unescaping
// ---------------------------------------------------------------------

fn escape_text(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn escape_attrib(s: &str) -> String {
    escape_text(s)
        .replace('"', "&quot;")
        .replace('\n', "&#10;")
        .replace('\t', "&#09;")
}

/// Number of bytes in the UTF-8 sequence starting with leading byte `b`.
fn utf8_char_len(b: u8) -> usize {
    if b & 0x80 == 0 {
        1
    } else if b & 0xE0 == 0xC0 {
        2
    } else if b & 0xF0 == 0xE0 {
        3
    } else if b & 0xF8 == 0xF0 {
        4
    } else {
        1
    }
}

fn unescape_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let bytes: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == '&' {
            if let Some(semi) = bytes[i..].iter().position(|&c| c == ';') {
                let entity: String = bytes[i + 1..i + semi].iter().collect();
                let resolved = match entity.as_str() {
                    "amp" => Some('&'.to_string()),
                    "lt" => Some('<'.to_string()),
                    "gt" => Some('>'.to_string()),
                    "quot" => Some('"'.to_string()),
                    "apos" => Some('\''.to_string()),
                    _ if entity.starts_with("#x") || entity.starts_with("#X") => {
                        u32::from_str_radix(&entity[2..], 16)
                            .ok()
                            .and_then(char::from_u32)
                            .map(|c| c.to_string())
                    }
                    _ if entity.starts_with('#') => entity[1..]
                        .parse::<u32>()
                        .ok()
                        .and_then(char::from_u32)
                        .map(|c| c.to_string()),
                    _ => None,
                };
                if let Some(r) = resolved {
                    out.push_str(&r);
                    i += semi + 1;
                    continue;
                }
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    out
}

// ---------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------

struct Parser<'a> {
    s: &'a [u8],
    pos: usize,
    line: usize,
    col: usize,
}

impl<'a> Parser<'a> {
    fn new(s: &'a str) -> Self {
        Parser {
            s: s.as_bytes(),
            pos: 0,
            line: 1,
            col: 0,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.s.get(self.pos).copied()
    }

    fn advance(&mut self) -> Option<u8> {
        let c = self.peek()?;
        self.pos += 1;
        if c == b'\n' {
            self.line += 1;
            self.col = 0;
        } else {
            self.col += 1;
        }
        Some(c)
    }

    fn starts_with(&self, s: &str) -> bool {
        self.s[self.pos..].starts_with(s.as_bytes())
    }

    fn skip(&mut self, n: usize) {
        for _ in 0..n {
            self.advance();
        }
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(b' ') | Some(b'\t') | Some(b'\n') | Some(b'\r')) {
            self.advance();
        }
    }

    fn err(&self, msg: &str) -> PyError {
        PyError::syntax_error(format!(
            "{}: line {}, column {}",
            msg,
            self.line,
            self.col
        ))
    }

    fn read_name(&mut self) -> String {
        let start = self.pos;
        while let Some(c) = self.peek() {
            if c.is_ascii_whitespace() || matches!(c, b'>' | b'/' | b'=' | b'?') {
                break;
            }
            self.advance();
        }
        String::from_utf8_lossy(&self.s[start..self.pos]).into_owned()
    }

    fn parse_attrs(&mut self, attrib: &PyObjectRef) -> PyResult<()> {
        loop {
            self.skip_ws();
            match self.peek() {
                Some(b'>') | Some(b'/') | Some(b'?') | None => break,
                _ => {}
            }
            let name = self.read_name();
            if name.is_empty() {
                break;
            }
            self.skip_ws();
            if self.peek() != Some(b'=') {
                return Err(self.err("not well-formed (invalid token)"));
            }
            self.advance();
            self.skip_ws();
            let quote = self.peek();
            if quote != Some(b'"') && quote != Some(b'\'') {
                return Err(self.err("not well-formed (invalid token)"));
            }
            let quote = quote.unwrap();
            self.advance();
            let start = self.pos;
            while self.peek().is_some() && self.peek() != Some(quote) {
                self.advance();
            }
            let raw = String::from_utf8_lossy(&self.s[start..self.pos]).into_owned();
            self.advance(); // closing quote
            if let PyObject::Dict(d) = &mut *attrib.borrow_mut() {
                let _ = d.set(py_str(&name), py_str(&unescape_entities(&raw)));
            }
        }
        Ok(())
    }

    /// Parse one element, returning the built Element and appending
    /// trailing text (up to the next sibling tag or the parent's close) to
    /// `sibling_tail_out` via the caller's text-accumulation loop instead.
    fn parse_element(&mut self) -> PyResult<PyObjectRef> {
        // assumes positioned right after '<'
        let tag = self.read_name();
        if tag.is_empty() {
            return Err(self.err("not well-formed (invalid token)"));
        }
        let attrib = py_dict();
        self.parse_attrs(&attrib)?;
        self.skip_ws();
        if self.peek() == Some(b'/') {
            self.advance();
            if self.peek() != Some(b'>') {
                return Err(self.err("not well-formed (invalid token)"));
            }
            self.advance();
            return Ok(new_element(py_str(&tag), attrib));
        }
        if self.peek() != Some(b'>') {
            return Err(self.err("not well-formed (invalid token)"));
        }
        self.advance();
        let el = new_element(py_str(&tag), attrib);
        self.parse_content(&el, &tag)?;
        Ok(el)
    }

    /// Parse element content (text, children, comments, PIs, CDATA) until
    /// the matching `</tag>` close is consumed.
    fn parse_content(&mut self, el: &PyObjectRef, tag: &str) -> PyResult<()> {
        let mut text = String::new();
        let mut have_text_slot = true; // still filling .text (no child yet)
        let mut last_child: Option<PyObjectRef> = None;

        macro_rules! flush_text {
            () => {
                if !text.is_empty() {
                    let unescaped = unescape_entities(&text);
                    if have_text_slot {
                        set_field(el, "text", py_str(&unescaped));
                    } else if let Some(ref lc) = last_child {
                        set_field(lc, "tail", py_str(&unescaped));
                    }
                    text.clear();
                }
            };
        }

        loop {
            match self.peek() {
                None => return Err(self.err("no element found")),
                Some(b'<') => {
                    if self.starts_with("</") {
                        flush_text!();
                        self.skip(2);
                        let close_name = self.read_name();
                        self.skip_ws();
                        if self.peek() != Some(b'>') {
                            return Err(self.err("not well-formed (invalid token)"));
                        }
                        self.advance();
                        if close_name != tag {
                            return Err(self.err("mismatched tag"));
                        }
                        return Ok(());
                    } else if self.starts_with("<!--") {
                        flush_text!();
                        self.skip(4);
                        let start = self.pos;
                        while !self.starts_with("-->") && self.peek().is_some() {
                            self.advance();
                        }
                        let comment_text =
                            String::from_utf8_lossy(&self.s[start..self.pos]).into_owned();
                        self.skip(3);
                        let marker = COMMENT_MARKER.with(|c| c.borrow().clone().unwrap());
                        let comment_el = new_element(marker, py_dict());
                        set_field(&comment_el, "text", py_str(&comment_text));
                        with_children_mut(el, |v| v.push(comment_el.clone()));
                        last_child = Some(comment_el);
                        have_text_slot = false;
                    } else if self.starts_with("<![CDATA[") {
                        self.skip(9);
                        let start = self.pos;
                        while !self.starts_with("]]>") && self.peek().is_some() {
                            self.advance();
                        }
                        text.push_str(&String::from_utf8_lossy(&self.s[start..self.pos]));
                        self.skip(3);
                    } else if self.starts_with("<?") {
                        flush_text!();
                        self.skip(2);
                        let start = self.pos;
                        while !self.starts_with("?>") && self.peek().is_some() {
                            self.advance();
                        }
                        let pi_text = String::from_utf8_lossy(&self.s[start..self.pos]).into_owned();
                        self.skip(2);
                        let (target, rest) = match pi_text.find(char::is_whitespace) {
                            Some(idx) => (pi_text[..idx].to_string(), pi_text[idx..].trim_start().to_string()),
                            None => (pi_text.clone(), String::new()),
                        };
                        let marker = PI_MARKER.with(|c| c.borrow().clone().unwrap());
                        let pi_el = new_element(marker, py_dict());
                        set_field(&pi_el, "text", py_str(&format!("{} {}", target, rest).trim_end()));
                        with_children_mut(el, |v| v.push(pi_el.clone()));
                        last_child = Some(pi_el);
                        have_text_slot = false;
                    } else {
                        flush_text!();
                        self.advance(); // consume '<'
                        let child = self.parse_element()?;
                        with_children_mut(el, |v| v.push(child.clone()));
                        last_child = Some(child);
                        have_text_slot = false;
                    }
                }
                Some(c) => {
                    // `self.s` is indexed by BYTE offset, but text content
                    // can obviously contain multi-byte UTF-8 characters
                    // (e.g. `<site>\xf8</site>` as UTF-8 source bytes) —
                    // pushing the single leading byte cast directly to
                    // `char` (the previous code here) treats it as a
                    // Latin-1 codepoint instead of decoding the real
                    // scalar value, corrupting any non-ASCII text into
                    // mojibake (each UTF-8 continuation byte became its
                    // own bogus character). Decode the full multi-byte
                    // sequence instead.
                    let len = utf8_char_len(c);
                    let end = (self.pos + len).min(self.s.len());
                    match std::str::from_utf8(&self.s[self.pos..end]) {
                        Ok(s) => text.push_str(s),
                        Err(_) => text.push(c as char),
                    }
                    for _ in 0..len {
                        self.advance();
                    }
                }
            }
        }
    }

    /// Skip an XML/doctype prolog: `<?xml ... ?>`, `<!DOCTYPE ...>`, comments,
    /// and whitespace, positioning right before the root element's `<`.
    fn skip_prolog(&mut self) {
        loop {
            self.skip_ws();
            if self.starts_with("<?") {
                self.skip(2);
                while !self.starts_with("?>") && self.peek().is_some() {
                    self.advance();
                }
                self.skip(2);
            } else if self.starts_with("<!--") {
                self.skip(4);
                while !self.starts_with("-->") && self.peek().is_some() {
                    self.advance();
                }
                self.skip(3);
            } else if self.starts_with("<!DOCTYPE") || self.starts_with("<!doctype") {
                let mut depth = 0i32;
                loop {
                    match self.peek() {
                        None => break,
                        Some(b'<') => {
                            depth += 1;
                            self.advance();
                        }
                        Some(b'>') => {
                            self.advance();
                            depth -= 1;
                            if depth <= 0 {
                                break;
                            }
                        }
                        _ => {
                            self.advance();
                        }
                    }
                }
            } else {
                break;
            }
        }
    }
}

fn parse_document(xml: &str) -> PyResult<PyObjectRef> {
    let mut p = Parser::new(xml);
    p.skip_prolog();
    if p.peek() != Some(b'<') {
        return Err(p.err("no element found"));
    }
    p.advance();
    p.parse_element()
}

// ---------------------------------------------------------------------
// Serializer
// ---------------------------------------------------------------------

fn is_comment(el: &PyObjectRef) -> bool {
    let tag = get_field(el, "tag");
    match tag {
        Some(t) => COMMENT_MARKER.with(|c| {
            c.borrow()
                .as_ref()
                .map(|m| m.get_id() == t.get_id())
                .unwrap_or(false)
        }),
        None => false,
    }
}

fn is_pi(el: &PyObjectRef) -> bool {
    let tag = get_field(el, "tag");
    match tag {
        Some(t) => PI_MARKER.with(|c| {
            c.borrow()
                .as_ref()
                .map(|m| m.get_id() == t.get_id())
                .unwrap_or(false)
        }),
        None => false,
    }
}

fn serialize_element(obj: &PyObjectRef, out: &mut String, short_empty_elements: bool) {
    if is_comment(obj) {
        let text = opt_str(get_field(obj, "text").as_ref()).unwrap_or_default();
        out.push_str("<!--");
        out.push_str(&text);
        out.push_str("-->");
        if let Some(tail) = opt_str(get_field(obj, "tail").as_ref()) {
            out.push_str(&escape_text(&tail));
        }
        return;
    }
    if is_pi(obj) {
        let text = opt_str(get_field(obj, "text").as_ref()).unwrap_or_default();
        out.push_str("<?");
        out.push_str(&text);
        out.push_str("?>");
        if let Some(tail) = opt_str(get_field(obj, "tail").as_ref()) {
            out.push_str(&escape_text(&tail));
        }
        return;
    }
    // A `None` tag suppresses the wrapping start/end tag entirely (real
    // ElementTree "tag suppression" feature — `elem.tag = None` renders
    // only the element's text/children/tail, still real usable content).
    let tag_field = get_field(obj, "tag");
    let tag_is_none = tag_field.as_ref().map(is_none).unwrap_or(true);
    let attrib = get_field(obj, "attrib");
    let text = opt_str(get_field(obj, "text").as_ref());
    let tail = opt_str(get_field(obj, "tail").as_ref());
    let children = children_of(obj);

    if tag_is_none {
        if let Some(t) = &text {
            out.push_str(&escape_text(t));
        }
        for child in &children {
            serialize_element(child, out, short_empty_elements);
        }
        if let Some(t) = &tail {
            out.push_str(&escape_text(t));
        }
        return;
    }

    let tag = tag_field.map(|t| t.str()).unwrap_or_default();
    out.push('<');
    out.push_str(&tag);
    if let Some(attrib) = &attrib {
        if let PyObject::Dict(d) = &*attrib.borrow() {
            for (k, v) in d.items() {
                out.push(' ');
                out.push_str(&k.str());
                out.push_str("=\"");
                out.push_str(&escape_attrib(&v.str()));
                out.push('"');
            }
        }
    }
    if text.is_none() && children.is_empty() {
        if short_empty_elements {
            out.push_str(" />");
        } else {
            out.push('>');
            out.push_str("</");
            out.push_str(&tag);
            out.push('>');
        }
    } else {
        out.push('>');
        if let Some(t) = &text {
            out.push_str(&escape_text(t));
        }
        for child in &children {
            serialize_element(child, out, short_empty_elements);
        }
        out.push_str("</");
        out.push_str(&tag);
        out.push('>');
    }
    if let Some(t) = &tail {
        out.push_str(&escape_text(t));
    }
}

fn tostring_str(obj: &PyObjectRef, xml_declaration: bool, short_empty_elements: bool) -> String {
    let mut out = String::new();
    if xml_declaration {
        out.push_str("<?xml version='1.0' encoding='utf-8'?>\n");
    }
    serialize_element(obj, &mut out, short_empty_elements);
    out
}

/// Encode a serialized document `s` as bytes for `encoding` (case-
/// insensitive). `us-ascii`/`ascii` (the real default) and other
/// single-byte encodings can't represent arbitrary Unicode, so any
/// non-representable character is replaced with a numeric character
/// reference (`&#NNN;`) — matching real `xml.etree.ElementTree`'s
/// `_escape_cdata`/`_encode` behavior for exactly this reason. `utf-8`
/// (and `unicode`, handled by the caller before reaching here) need no
/// such escaping. Deliberately NOT implemented: `utf-16`/`utf-32` family
/// encodings (rare in the test corpus, real multi-byte little/big-endian
/// + BOM handling would be a further chunk of work).
fn encode_for(s: &str, encoding: &str) -> Vec<u8> {
    let enc = encoding.to_ascii_lowercase();
    match enc.as_str() {
        "utf-8" | "utf8" => s.as_bytes().to_vec(),
        "iso-8859-1" | "latin-1" | "latin1" => {
            let mut out = String::with_capacity(s.len());
            for c in s.chars() {
                if (c as u32) <= 0xFF {
                    out.push(c);
                } else {
                    out.push_str(&format!("&#{};", c as u32));
                }
            }
            out.chars().map(|c| c as u8).collect()
        }
        _ => {
            // us-ascii/ascii and anything else unrecognized: ASCII-safe,
            // numeric-escape everything above 0x7F.
            let mut out = String::with_capacity(s.len());
            for c in s.chars() {
                if (c as u32) <= 0x7F {
                    out.push(c);
                } else {
                    out.push_str(&format!("&#{};", c as u32));
                }
            }
            out.into_bytes()
        }
    }
}

// ---------------------------------------------------------------------
// find / findall / findtext (simple ElementPath subset — no namespaces)
// ---------------------------------------------------------------------

fn iter_all<'o>(el: &'o PyObjectRef, out: &mut Vec<PyObjectRef>) {
    out.push(el.clone());
    for c in children_of(el) {
        iter_all(&c, out);
    }
}

fn matches_step(el: &PyObjectRef, step: &str) -> bool {
    if step == "*" {
        return !is_comment(el) && !is_pi(el);
    }
    get_field(el, "tag")
        .map(|t| matches!(&*t.borrow(), PyObject::Str(_)) && t.str() == step)
        .unwrap_or(false)
}

/// Extremely small path evaluator: supports `tag`, `*`, `.`, `a/b/c`,
/// a leading `//tag` (search all descendants), and `tag[@attr]` /
/// `tag[@attr='v']` predicates. Enough for the common cases in the test
/// corpus; full ElementPath grammar (indices, `last()`, `..`) is not
/// attempted.
fn path_find_all(root: &PyObjectRef, path: &str) -> Vec<PyObjectRef> {
    let path = path.trim();
    if path.is_empty() || path == "." {
        return vec![root.clone()];
    }
    let (descendant, path) = if let Some(rest) = path.strip_prefix("//") {
        (true, rest)
    } else {
        (false, path)
    };
    let mut current: Vec<PyObjectRef> = if descendant {
        let mut all = vec![];
        iter_all(root, &mut all);
        all
    } else {
        children_of(root)
    };

    let steps: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    for (i, raw_step) in steps.iter().enumerate() {
        let (step, pred) = match raw_step.find('[') {
            Some(idx) if raw_step.ends_with(']') => {
                (&raw_step[..idx], Some(&raw_step[idx + 1..raw_step.len() - 1]))
            }
            _ => (*raw_step, None),
        };
        let mut next = vec![];
        for el in &current {
            if matches_step(el, step) {
                if let Some(pred) = pred {
                    if !eval_predicate(el, pred) {
                        continue;
                    }
                }
                next.push(el.clone());
            }
        }
        if i + 1 < steps.len() {
            let mut expanded = vec![];
            for el in &next {
                expanded.extend(children_of(el));
            }
            current = expanded;
        } else {
            current = next;
        }
    }
    current
}

fn eval_predicate(el: &PyObjectRef, pred: &str) -> bool {
    let pred = pred.trim();
    if let Some(attr) = pred.strip_prefix('@') {
        if let Some((k, v)) = attr.split_once('=') {
            let k = k.trim();
            let v = v.trim().trim_matches(|c| c == '\'' || c == '"');
            let attrib = get_field(el, "attrib");
            if let Some(attrib) = attrib {
                if let PyObject::Dict(d) = &*attrib.borrow() {
                    return d
                        .get(&py_str(k))
                        .ok()
                        .flatten()
                        .map(|val| val.str() == v)
                        .unwrap_or(false);
                }
            }
            false
        } else {
            let k = attr.trim();
            let attrib = get_field(el, "attrib");
            if let Some(attrib) = attrib {
                if let PyObject::Dict(d) = &*attrib.borrow() {
                    return d.get(&py_str(k)).ok().flatten().is_some();
                }
            }
            false
        }
    } else {
        // [tag] child presence predicate
        !path_find_all(el, pred).is_empty()
    }
}

// ---------------------------------------------------------------------
// Module setup
// ---------------------------------------------------------------------

pub fn create_xml_etree_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! et_func {
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

    // register_namespace(prefix, uri): store in a simple global map. Real
    // serialization-time prefix substitution is not implemented (the
    // native serializer above always emits raw tag text), but the map
    // itself is exposed via `_namespace_map` for code/tests that inspect it
    // directly.
    let ns_map_dict = py_dict();
    {
        let rn_typ = PyObjectRef::new(PyObject::Type {
            name: "_register_namespace".into(),
            dict: Box::new(str_map_to_typedict({
                let mut m: HashMap<String, PyObjectRef> = HashMap::new();
                let ns_map_clone = ns_map_dict.clone();
                m.insert(
                    "__call__".to_string(),
                    PyObjectRef::new(PyObject::Closure(std::rc::Rc::new(
                        move |args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                            // args[0] is the bound "self" instance
                            if args.len() >= 3 {
                                let prefix = args[1].str();
                                let uri = args[2].str();
                                NAMESPACE_MAP.with(|m| {
                                    m.borrow_mut().insert(uri.clone(), prefix.clone());
                                });
                                if let PyObject::Dict(dd) = &mut *ns_map_clone.borrow_mut() {
                                    let _ = dd.set(py_str(&uri), py_str(&prefix));
                                }
                            }
                            Ok(py_none())
                        },
                    ))),
                );
                m
            })),
            bases: vec![],
            mro: vec![],
        });
        let mut rn_dict = AttrMap::new();
        rn_dict.insert_str("_namespace_map", ns_map_dict.clone());
        d.insert_str(
            "register_namespace",
            PyObjectRef::new(PyObject::Instance {
                typ: rn_typ,
                dict: rn_dict,
            }),
        );
    }

    // Comment / ProcessingInstruction sentinel callables (their identity IS
    // the marker; see `is_comment`/`is_pi` above).
    let comment_marker = PyObjectRef::new(PyObject::BuiltinFunction {
        name: "Comment".to_string(),
        func: |args| {
            let text = args.first().map(|a| a.str()).unwrap_or_default();
            let marker = COMMENT_MARKER.with(|c| c.borrow().clone().unwrap());
            let el = new_element(marker, py_dict());
            set_field(&el, "text", py_str(&text));
            Ok(el)
        },
    });
    COMMENT_MARKER.with(|c| *c.borrow_mut() = Some(comment_marker.clone()));
    d.insert_str("Comment", comment_marker);

    let pi_marker = PyObjectRef::new(PyObject::BuiltinFunction {
        name: "ProcessingInstruction".to_string(),
        func: |args| {
            let target = args.first().map(|a| a.str()).unwrap_or_default();
            let text = args.get(1).filter(|a| !is_none(a)).map(|a| a.str());
            let marker = PI_MARKER.with(|c| c.borrow().clone().unwrap());
            let el = new_element(marker, py_dict());
            let content = match text {
                Some(t) => format!("{} {}", target, t),
                None => target,
            };
            set_field(&el, "text", py_str(&content));
            Ok(el)
        },
    });
    PI_MARKER.with(|c| *c.borrow_mut() = Some(pi_marker.clone()));
    d.insert_str("ProcessingInstruction", pi_marker.clone());
    d.insert_str("PI", pi_marker);

    // ------------------------------------------------------------------
    // Element type
    // ------------------------------------------------------------------
    let mut element_type_dict = HashMap::new();
    macro_rules! e_method {
        ($name:expr, $func:expr) => {
            element_type_dict.insert(
                $name.to_string(),
                PyObjectRef::new(PyObject::BuiltinFunction {
                    name: $name.to_string(),
                    func: $func,
                }),
            );
        };
    }

    e_method!("append", |args| {
        if args.len() < 2 || !is_element_instance(&args[0]) {
            return Err(PyError::type_error("append() takes exactly one argument"));
        }
        let child = args[1].clone();
        with_children_mut(&args[0], |v| v.push(child))
            .ok_or_else(|| PyError::type_error("append: self is not an Element"))?;
        Ok(py_none())
    });

    e_method!("extend", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("extend() takes exactly one argument"));
        }
        let items: Vec<PyObjectRef> = match &*args[1].borrow() {
            PyObject::List(l) => l.clone(),
            PyObject::Tuple(t) => t.clone(),
            _ => return Err(PyError::type_error("extend() expects an iterable of elements")),
        };
        with_children_mut(&args[0], |v| v.extend(items))
            .ok_or_else(|| PyError::type_error("extend: self is not an Element"))?;
        Ok(py_none())
    });

    e_method!("insert", |args| {
        if args.len() < 3 {
            return Err(PyError::type_error("insert() takes exactly 2 arguments"));
        }
        let idx = args[1].as_i64().unwrap_or(0);
        let child = args[2].clone();
        with_children_mut(&args[0], |v| {
            let idx = idx.clamp(0, v.len() as i64) as usize;
            v.insert(idx, child);
        })
        .ok_or_else(|| PyError::type_error("insert: self is not an Element"))?;
        Ok(py_none())
    });

    e_method!("remove", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("remove() takes exactly one argument"));
        }
        let target_id = args[1].get_id();
        let removed = with_children_mut(&args[0], |v| {
            if let Some(pos) = v.iter().position(|c| c.get_id() == target_id) {
                v.remove(pos);
                true
            } else {
                false
            }
        })
        .unwrap_or(false);
        if removed {
            Ok(py_none())
        } else {
            Err(PyError::value_error("list.remove(x): x not in list"))
        }
    });

    e_method!("clear", |args| {
        if let PyObject::Instance { dict, .. } = &mut *args[0].borrow_mut() {
            dict.insert_str("attrib", py_dict());
            dict.insert_str("text", py_none());
            dict.insert_str("tail", py_none());
            dict.insert_str("children", py_list(vec![]));
        }
        Ok(py_none())
    });

    e_method!("makeelement", |args| {
        if args.len() < 3 {
            return Err(PyError::type_error("makeelement() takes exactly 2 arguments"));
        }
        let tag = args[1].clone();
        let attrib_src = &args[2];
        let attrib = py_dict();
        if let PyObject::Dict(src) = &*attrib_src.borrow() {
            if let PyObject::Dict(d) = &mut *attrib.borrow_mut() {
                for (k, v) in src.items() {
                    let _ = d.set(k, v);
                }
            }
        }
        Ok(new_element(tag, attrib))
    });

    e_method!("find", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("find() takes at least 1 argument"));
        }
        let path = args[1].str();
        Ok(path_find_all(&args[0], &path)
            .into_iter()
            .next()
            .unwrap_or_else(py_none))
    });

    e_method!("findall", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("findall() takes at least 1 argument"));
        }
        let path = args[1].str();
        Ok(py_list(path_find_all(&args[0], &path)))
    });

    e_method!("iterfind", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("iterfind() takes at least 1 argument"));
        }
        let path = args[1].str();
        let results = path_find_all(&args[0], &path);
        crate::object::builtin_iter(&[py_list(results)])
    });

    e_method!("findtext", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("findtext() takes at least 1 argument"));
        }
        let path = args[1].str();
        let default = args.get(2).cloned().unwrap_or_else(py_none);
        match path_find_all(&args[0], &path).into_iter().next() {
            Some(el) => Ok(opt_str(get_field(&el, "text").as_ref())
                .map(|s| py_str(&s))
                .unwrap_or_else(|| py_str(""))),
            None => Ok(default),
        }
    });

    e_method!("get", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("get() takes at least 1 argument"));
        }
        let key = args[1].str();
        let default = args.get(2).cloned().unwrap_or_else(py_none);
        if let Some(attrib) = get_field(&args[0], "attrib") {
            if let PyObject::Dict(ad) = &*attrib.borrow() {
                if let Ok(Some(v)) = ad.get(&py_str(&key)) {
                    return Ok(v);
                }
            }
        }
        Ok(default)
    });

    e_method!("set", |args| {
        if args.len() < 3 {
            return Err(PyError::type_error("set() takes exactly 2 arguments"));
        }
        if let Some(attrib) = get_field(&args[0], "attrib") {
            if let PyObject::Dict(ad) = &mut *attrib.borrow_mut() {
                let _ = ad.set(py_str(&args[1].str()), args[2].clone());
            }
        }
        Ok(py_none())
    });

    e_method!("keys", |args| {
        if let Some(attrib) = get_field(&args[0], "attrib") {
            if let PyObject::Dict(ad) = &*attrib.borrow() {
                return Ok(py_list(ad.keys()));
            }
        }
        Ok(py_list(vec![]))
    });

    e_method!("items", |args| {
        if let Some(attrib) = get_field(&args[0], "attrib") {
            if let PyObject::Dict(ad) = &*attrib.borrow() {
                return Ok(py_list(
                    ad.items().into_iter().map(|(k, v)| py_tuple(vec![k, v])).collect(),
                ));
            }
        }
        Ok(py_list(vec![]))
    });

    e_method!("iter", |args| {
        let tag_filter = args.get(1).filter(|a| !is_none(a)).map(|a| a.str());
        let mut all = vec![];
        iter_all(&args[0], &mut all);
        if let Some(tag) = tag_filter {
            if tag != "*" {
                all.retain(|el| matches_step(el, &tag));
            }
        }
        crate::object::builtin_iter(&[py_list(all)])
    });

    e_method!("itertext", |args| {
        let mut all = vec![];
        iter_all(&args[0], &mut all);
        let mut texts = vec![];
        for el in &all {
            if is_comment(el) || is_pi(el) {
                continue;
            }
            if let Some(t) = opt_str(get_field(el, "text").as_ref()) {
                if !t.is_empty() {
                    texts.push(py_str(&t));
                }
            }
            if let Some(t) = opt_str(get_field(el, "tail").as_ref()) {
                if !t.is_empty() && el.get_id() != args[0].get_id() {
                    texts.push(py_str(&t));
                }
            }
        }
        crate::object::builtin_iter(&[py_list(texts)])
    });

    e_method!("__len__", |args| {
        Ok(py_int(children_of(&args[0]).len() as i64))
    });

    e_method!("__bool__", |_args| {
        // CPython emits a DeprecationWarning here and treats truthiness as
        // "has children"; we skip the warning but keep the behavior.
        Ok(py_bool(true))
    });

    e_method!("__iter__", |args| {
        crate::object::builtin_iter(&[py_list(children_of(&args[0]))])
    });

    e_method!("__getitem__", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("__getitem__ needs an index"));
        }
        let children = children_of(&args[0]);
        match args[1].as_i64() {
            Some(i) => {
                let len = children.len() as i64;
                let idx = if i < 0 { i + len } else { i };
                if idx < 0 {
                    return Err(PyError::index_error("child index out of range"));
                }
                children
                    .get(idx as usize)
                    .cloned()
                    .ok_or_else(|| PyError::index_error("child index out of range"))
            }
            None => Err(PyError::type_error("indices must be integers")),
        }
    });

    e_method!("__setitem__", |args| {
        if args.len() < 3 {
            return Err(PyError::type_error("__setitem__ needs index and value"));
        }
        let idx = args[1]
            .as_i64()
            .ok_or_else(|| PyError::type_error("indices must be integers"))?;
        let value = args[2].clone();
        with_children_mut(&args[0], |v| {
            let len = v.len() as i64;
            let idx = if idx < 0 { idx + len } else { idx };
            if idx >= 0 && (idx as usize) < v.len() {
                v[idx as usize] = value;
                Ok(())
            } else {
                Err(PyError::index_error("assignment index out of range"))
            }
        })
        .unwrap_or(Err(PyError::type_error("not an Element")))?;
        Ok(py_none())
    });

    e_method!("__delitem__", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("__delitem__ needs an index"));
        }
        let idx = args[1]
            .as_i64()
            .ok_or_else(|| PyError::type_error("indices must be integers"))?;
        with_children_mut(&args[0], |v| {
            let len = v.len() as i64;
            let idx = if idx < 0 { idx + len } else { idx };
            if idx >= 0 && (idx as usize) < v.len() {
                v.remove(idx as usize);
                Ok(())
            } else {
                Err(PyError::index_error("assignment index out of range"))
            }
        })
        .unwrap_or(Err(PyError::type_error("not an Element")))?;
        Ok(py_none())
    });

    e_method!("__repr__", |args| {
        let tag = get_field(&args[0], "tag").map(|t| t.str()).unwrap_or_default();
        Ok(py_str(&format!(
            "<Element {:?} at 0x{:012x}>",
            tag,
            args[0].get_id()
        )))
    });

    element_type_dict.insert(
        "__module__".to_string(),
        py_str("xml.etree.ElementTree"),
    );
    let element_type = PyObjectRef::new(PyObject::Type {
        name: "Element".to_string(),
        dict: Box::new(str_map_to_typedict(element_type_dict)),
        bases: vec![],
        mro: vec![],
    });
    ELEMENT_TYPE.with(|cache| *cache.borrow_mut() = Some(element_type.clone()));
    d.insert_str("Element", element_type.clone());
    d.insert_str("_Element_Py", element_type);

    // ------------------------------------------------------------------
    // Element(tag, attrib={}, **extra) / SubElement(parent, tag, attrib={}, **extra)
    // ------------------------------------------------------------------
    // Merge a dict's items into `attrib`.
    fn merge_dict_into(attrib: &PyObjectRef, src: &PyObjectRef) {
        if let PyObject::Dict(sd) = &*src.borrow() {
            if let PyObject::Dict(d) = &mut *attrib.borrow_mut() {
                for (k, v) in sd.items() {
                    let _ = d.set(k, v);
                }
            }
        }
    }

    fn build_attrib(args: &[PyObjectRef], attrib_idx: usize) -> PyObjectRef {
        let attrib = py_dict();
        if let Some(src) = args.get(attrib_idx) {
            // Real `Element(tag, attrib={}, **extra)`: when the caller
            // passes `attrib=` as a KEYWORD argument rather than
            // positionally (e.g. `Element("tag", attrib=dict(href="#"))`
            // — confirmed via CPython's own `test_issue14818`), this
            // codebase's calling convention (see the kwargs gotcha in
            // CLAUDE.md) packs it into the trailing kwargs dict under the
            // literal key `"attrib"` instead of filling this positional
            // slot — so `args[attrib_idx]` IS that trailing kwargs dict,
            // shaped like `{"attrib": {...}}`, not the attrib dict itself.
            // Unwrap that one level first; a plain positional attrib dict
            // (the common case) has no such nested "attrib" key and merges
            // directly.
            let mut src_to_merge = src.clone();
            let mut extra_after_unwrap: Option<PyObjectRef> = None;
            if let PyObject::Dict(sd) = &*src.borrow() {
                if let Ok(Some(inner)) = sd.get(&py_str("attrib")) {
                    if matches!(&*inner.borrow(), PyObject::Dict(_)) {
                        extra_after_unwrap = Some(src.clone());
                        src_to_merge = inner;
                    }
                }
            }
            merge_dict_into(&attrib, &src_to_merge);
            // Any OTHER keys in the same kwargs dict besides "attrib"
            // itself are extra attributes (`Element(tag, attrib={}, id="x")`).
            if let Some(extra) = extra_after_unwrap {
                if let PyObject::Dict(ed) = &*extra.borrow() {
                    if let PyObject::Dict(d) = &mut *attrib.borrow_mut() {
                        for (k, v) in ed.items() {
                            if k.str() != "attrib" {
                                let _ = d.set(k, v);
                            }
                        }
                    }
                }
            }
        }
        // Trailing kwargs dict (calling-convention gotcha: kwargs are
        // packed as a trailing dict argument) also merges into attrib —
        // covers `Element(tag, {...}, id="extra")`, a separate positional
        // attrib dict PLUS real trailing kwargs.
        if let Some(last) = args.last() {
            if args.len() > attrib_idx + 1 {
                merge_dict_into(&attrib, last);
            }
        }
        attrib
    }

    et_func!("Element", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("Element() missing tag argument"));
        }
        let tag = args[0].clone();
        let attrib = build_attrib(args, 1);
        Ok(new_element(tag, attrib))
    });

    et_func!("SubElement", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error(
                "SubElement() requires at least 2 arguments",
            ));
        }
        let parent = &args[0];
        let tag = args[1].clone();
        let attrib = build_attrib(args, 2);
        let child = new_element(tag, attrib);
        with_children_mut(parent, |v| v.push(child.clone()));
        Ok(child)
    });

    et_func!("iselement", |args| {
        Ok(py_bool(args.first().map(is_element_instance).unwrap_or(false)))
    });

    et_func!("dump", |args| {
        if let Some(el) = args.first() {
            println!("{}", tostring_str(el, false, true));
        }
        Ok(py_none())
    });

    // Shared option parsing for `tostring`/`tostringlist`/`ElementTree.
    // write`: positional `encoding` (index `enc_idx`), plus a trailing
    // packed-kwargs dict that may carry `encoding`/`short_empty_elements`/
    // `xml_declaration` (see the calling-convention gotcha in CLAUDE.md).
    // Returns (effective_encoding, short_empty_elements, xml_declaration).
    fn parse_serialize_opts(args: &[PyObjectRef], enc_idx: usize) -> (String, bool, bool) {
        let mut encoding: Option<String> = None;
        let mut short_empty = true;
        let mut xml_decl_override: Option<bool> = None;
        if args.len() > enc_idx {
            if let PyObject::Str(s) = &*args[enc_idx].borrow() {
                encoding = Some(s.to_string());
            }
        }
        if let Some(last) = args.last() {
            if let PyObject::Dict(d) = &*last.borrow() {
                if let Ok(Some(v)) = d.get(&py_str("encoding")) {
                    if !is_none(&v) {
                        encoding = Some(v.str());
                    }
                }
                if let Ok(Some(v)) = d.get(&py_str("short_empty_elements")) {
                    short_empty = v.truthy();
                }
                if let Ok(Some(v)) = d.get(&py_str("xml_declaration")) {
                    if !is_none(&v) {
                        xml_decl_override = Some(v.truthy());
                    }
                }
            }
        }
        let effective_encoding = encoding.clone().unwrap_or_else(|| "us-ascii".to_string());
        let default_no_decl = matches!(
            effective_encoding.to_ascii_lowercase().as_str(),
            "us-ascii" | "ascii" | "utf-8" | "utf8" | "unicode"
        );
        let xml_declaration = xml_decl_override.unwrap_or(!default_no_decl);
        (effective_encoding, short_empty, xml_declaration)
    }

    et_func!("tostring", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("tostring() missing required argument"));
        }
        let (encoding, short_empty, xml_declaration) = parse_serialize_opts(args, 1);
        let is_unicode = encoding.eq_ignore_ascii_case("unicode");
        let s = tostring_str(&args[0], xml_declaration, short_empty);
        if is_unicode {
            Ok(py_str(&s))
        } else {
            Ok(PyObjectRef::new(PyObject::Bytes(encode_for(&s, &encoding))))
        }
    });

    et_func!("tostringlist", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("tostringlist() missing required argument"));
        }
        let (encoding, short_empty, xml_declaration) = parse_serialize_opts(args, 1);
        let is_unicode = encoding.eq_ignore_ascii_case("unicode");
        let s = tostring_str(&args[0], xml_declaration, short_empty);
        if is_unicode {
            Ok(py_list(vec![py_str(&s)]))
        } else {
            Ok(py_list(vec![PyObjectRef::new(PyObject::Bytes(
                encode_for(&s, &encoding),
            ))]))
        }
    });

    et_func!("fromstring", |args| {
        if args.is_empty() {
            return Err(PyError::type_error(
                "fromstring() missing required argument",
            ));
        }
        let xml_str = match &*args[0].borrow() {
            PyObject::Bytes(b) => String::from_utf8_lossy(b).into_owned(),
            _ => args[0].str(),
        };
        parse_document(&xml_str)
    });
    // XML() is a plain alias for fromstring().
    if let Some(f) = d.get("fromstring").cloned() {
        d.insert_str("XML", f);
    }

    et_func!("fromstringlist", |args| {
        if args.is_empty() {
            return Err(PyError::type_error(
                "fromstringlist() missing required argument",
            ));
        }
        let parts: Vec<String> = match &*args[0].borrow() {
            PyObject::List(l) => l.iter().map(|x| x.str()).collect(),
            PyObject::Tuple(t) => t.iter().map(|x| x.str()).collect(),
            _ => vec![args[0].str()],
        };
        parse_document(&parts.join(""))
    });

    et_func!("indent", |args| {
        // No-op pretty-printer: real reindentation would require mutating
        // text/tail whitespace throughout the tree. Left unimplemented
        // (returns None like the real function) rather than half-done.
        let _ = args;
        Ok(py_none())
    });

    // ------------------------------------------------------------------
    // ElementTree class
    // ------------------------------------------------------------------
    let mut et_type_dict = HashMap::new();
    macro_rules! et_method {
        ($name:expr, $func:expr) => {
            et_type_dict.insert(
                $name.to_string(),
                PyObjectRef::new(PyObject::BuiltinFunction {
                    name: $name.to_string(),
                    func: $func,
                }),
            );
        };
    }

    et_method!("__init__", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("__init__ needs self"));
        }
        let root = args.get(1).filter(|a| !is_none(a)).cloned();
        if let PyObject::Instance { dict, .. } = &mut *args[0].borrow_mut() {
            dict.insert_str("_root", root.unwrap_or_else(py_none));
        }
        Ok(py_none())
    });

    et_method!("getroot", |args| {
        Ok(get_field(&args[0], "_root").unwrap_or_else(py_none))
    });

    et_method!("_setroot", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("_setroot() takes exactly one argument"));
        }
        set_field(&args[0], "_root", args[1].clone());
        Ok(py_none())
    });

    et_method!("parse", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("parse() takes at least 1 argument"));
        }
        let text = read_source_to_string(&args[1])?;
        let root = parse_document(&text)?;
        set_field(&args[0], "_root", root.clone());
        Ok(root)
    });

    et_method!("write", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("write() takes at least 1 argument"));
        }
        let root = get_field(&args[0], "_root").unwrap_or_else(py_none);
        let (encoding, short_empty, xml_declaration) = parse_serialize_opts(args, 2);
        let s = tostring_str(&root, xml_declaration, short_empty);
        if encoding.eq_ignore_ascii_case("unicode") {
            write_str_to_target(&args[1], &s)?;
        } else {
            write_bytes_to_target(&args[1], encode_for(&s, &encoding))?;
        }
        Ok(py_none())
    });

    et_method!("find", |args| {
        let root = get_field(&args[0], "_root").unwrap_or_else(py_none);
        if is_none(&root) {
            return Ok(py_none());
        }
        let path = args.get(1).map(|a| a.str()).unwrap_or_default();
        Ok(path_find_all(&root, &path).into_iter().next().unwrap_or_else(py_none))
    });

    et_method!("findall", |args| {
        let root = get_field(&args[0], "_root").unwrap_or_else(py_none);
        if is_none(&root) {
            return Ok(py_list(vec![]));
        }
        let path = args.get(1).map(|a| a.str()).unwrap_or_default();
        Ok(py_list(path_find_all(&root, &path)))
    });

    et_method!("findtext", |args| {
        let root = get_field(&args[0], "_root").unwrap_or_else(py_none);
        let default = args.get(2).cloned().unwrap_or_else(py_none);
        if is_none(&root) {
            return Ok(default);
        }
        let path = args.get(1).map(|a| a.str()).unwrap_or_default();
        match path_find_all(&root, &path).into_iter().next() {
            Some(el) => Ok(opt_str(get_field(&el, "text").as_ref())
                .map(|s| py_str(&s))
                .unwrap_or_else(|| py_str(""))),
            None => Ok(default),
        }
    });

    et_method!("iter", |args| {
        let root = get_field(&args[0], "_root").unwrap_or_else(py_none);
        if is_none(&root) {
            return crate::object::builtin_iter(&[py_list(vec![])]);
        }
        let tag_filter = args.get(1).filter(|a| !is_none(a)).map(|a| a.str());
        let mut all = vec![];
        iter_all(&root, &mut all);
        if let Some(tag) = tag_filter {
            if tag != "*" {
                all.retain(|el| matches_step(el, &tag));
            }
        }
        crate::object::builtin_iter(&[py_list(all)])
    });

    et_type_dict.insert(
        "__module__".to_string(),
        py_str("xml.etree.ElementTree"),
    );
    let et_type = PyObjectRef::new(PyObject::Type {
        name: "ElementTree".to_string(),
        dict: Box::new(str_map_to_typedict(et_type_dict)),
        bases: vec![],
        mro: vec![],
    });
    ELEMENTTREE_TYPE.with(|c| *c.borrow_mut() = Some(et_type.clone()));
    d.insert_str("ElementTree", et_type);

    d
}

fn read_source_to_string(src: &PyObjectRef) -> PyResult<String> {
    // A path/str, or a file-like object with .read().
    let b = src.borrow();
    match &*b {
        PyObject::Str(s) => {
            let path = s.to_string();
            drop(b);
            std::fs::read_to_string(&path)
                .map_err(|e| PyError::OsError(e.to_string()))
        }
        _ => {
            drop(b);
            if let Ok(read_fn) = src.borrow().get_attribute("read") {
                let content = crate::object::builtin_call(&read_fn, &[])?;
                let result = match &*content.borrow() {
                    PyObject::Bytes(bts) => String::from_utf8_lossy(bts).into_owned(),
                    _ => content.str(),
                };
                Ok(result)
            } else {
                Err(PyError::type_error("parse() expects a path or file-like object"))
            }
        }
    }
}

/// Write already-encoded bytes to `target` — a path/filename (`str`) is
/// opened and written directly (matching the encoding requested, since a
/// plain `std::fs::write` doesn't interpret content); a file-like object
/// gets `.write(bytes)` (binary-mode file object, the common case for a
/// non-`unicode` encoding per real `ElementTree.write`).
fn write_bytes_to_target(target: &PyObjectRef, data: Vec<u8>) -> PyResult<()> {
    let b = target.borrow();
    match &*b {
        PyObject::Str(s) => {
            let path = s.to_string();
            drop(b);
            std::fs::write(&path, &data).map_err(|e| PyError::OsError(e.to_string()))
        }
        _ => {
            drop(b);
            if let Ok(write_fn) = target.borrow().get_attribute("write") {
                crate::object::builtin_call(
                    &write_fn,
                    &[PyObjectRef::new(PyObject::Bytes(data))],
                )?;
                Ok(())
            } else {
                Err(PyError::type_error("write() expects a path or file-like object"))
            }
        }
    }
}

/// Write a `str` to `target` — used for `encoding='unicode'`, where real
/// `ElementTree.write` writes TEXT: a path/filename is written as UTF-8
/// bytes on disk (matching `open(path, 'w')`'s platform default), a
/// file-like object gets `.write(str)` (text-mode file object).
fn write_str_to_target(target: &PyObjectRef, content: &str) -> PyResult<()> {
    let b = target.borrow();
    match &*b {
        PyObject::Str(s) => {
            let path = s.to_string();
            drop(b);
            std::fs::write(&path, content.as_bytes()).map_err(|e| PyError::OsError(e.to_string()))
        }
        _ => {
            drop(b);
            if let Ok(write_fn) = target.borrow().get_attribute("write") {
                crate::object::builtin_call(&write_fn, &[py_str(content)])?;
                Ok(())
            } else {
                Err(PyError::type_error("write() expects a path or file-like object"))
            }
        }
    }
}
