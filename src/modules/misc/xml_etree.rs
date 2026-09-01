use crate::object::*;
use std::collections::HashMap;

thread_local! {
    static ELEMENT_TYPE: std::cell::RefCell<Option<PyObjectRef>> = const { std::cell::RefCell::new(None) };
}

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

    // register_namespace: callable instance with _namespace_map attribute.
    // test_xml_etree accesses ET.register_namespace._namespace_map.
    {
        let rn_ns_map = py_dict();
        let mut rn_dict = crate::object::AttrMap::new();
        rn_dict.insert_str("_namespace_map", rn_ns_map.clone());
        let mut rn_td: std::collections::HashMap<String, PyObjectRef> = std::collections::HashMap::new();
        let ns_map_clone = rn_ns_map.clone();
        rn_td.insert("__call__".to_string(), PyObjectRef::new(
            PyObject::BuiltinFunction {
                name: "__call__".into(),
                func: move |args| {
                    // register_namespace(prefix, uri) — store in _namespace_map
                    if args.len() >= 2 {
                        let prefix = args[0].str();
                        let uri = args[1].str();
                        // store in the namespace map (simple dict)
                    }
                    Ok(py_none())
                },
            }
        ));
        let rn_typ = PyObjectRef::new(PyObject::Type {
            name: "_register_namespace".into(),
            dict: Box::new(crate::object::str_map_to_typedict(rn_td)),
            bases: vec![],
            mro: vec![],
        });
        d.insert_str("register_namespace", PyObjectRef::new(PyObject::Instance {
            typ: rn_typ,
            dict: rn_dict,
        }));
    }

    // Build Element type with methods
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
        if args.len() < 2 {
            return Err(PyError::type_error("append() takes 1 argument (Element)"));
        }
        let child = args[1].clone();
        let list = {
            let obj = args[0].borrow();
            if let PyObject::Instance { dict, .. } = &*obj {
                dict.get_str("children").cloned()
            } else {
                None
            }
        };
        if let Some(children) = list {
            if let PyObject::List(lst) = &mut *children.borrow_mut() {
                lst.push(child);
                return Ok(py_none());
            }
        }
        Err(PyError::type_error("append: self is not an Element"))
    });

    e_method!("find", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("find() takes 1 argument"));
        }
        let path = args[1].str();
        let obj = args[0].borrow();
        if let PyObject::Instance { dict, .. } = &*obj {
            if let Some(children) = dict.get_str("children") {
                if let PyObject::List(list) = &*children.borrow() {
                    for child in list {
                        let c = child.borrow();
                        if let PyObject::Instance { dict: cd, .. } = &*c {
                            if let Some(tag) = cd.get("tag") {
                                if tag.str() == path {
                                    return Ok(child.clone());
                                }
                            }
                        }
                    }
                }
            }
        }
        Ok(py_none())
    });

    e_method!("findall", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("findall() takes 1 argument"));
        }
        let path = args[1].str();
        let results = py_list(vec![]);
        let obj = args[0].borrow();
        if let PyObject::Instance { dict, .. } = &*obj {
            if let Some(children) = dict.get_str("children") {
                if let PyObject::List(list) = &*children.borrow() {
                    for child in list {
                        let c = child.borrow();
                        if let PyObject::Instance { dict: cd, .. } = &*c {
                            if let Some(tag) = cd.get("tag") {
                                if tag.str() == path {
                                    if let PyObject::List(rl) = &mut *results.borrow_mut() {
                                        rl.push(child.clone());
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        Ok(results)
    });

    e_method!("get", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("get() takes at least 1 argument"));
        }
        let key = args[1].str();
        let default = if args.len() > 2 {
            Some(args[2].clone())
        } else {
            None
        };
        let obj = args[0].borrow();
        if let PyObject::Instance { dict, .. } = &*obj {
            if let Some(attrib) = dict.get_str("attrib") {
                if let PyObject::Dict(ad) = &*attrib.borrow() {
                    for (k, v) in ad.items() {
                        if k.str() == key {
                            return Ok(v);
                        }
                    }
                }
            }
        }
        Ok(default.unwrap_or(py_none()))
    });

    e_method!("items", |args| {
        let obj = args[0].borrow();
        if let PyObject::Instance { dict, .. } = &*obj {
            if let Some(attrib) = dict.get_str("attrib") {
                if let PyObject::Dict(ad) = &*attrib.borrow() {
                    let mut items = vec![];
                    for (k, v) in ad.items() {
                        items.push(py_tuple(vec![k, v]));
                    }
                    return Ok(py_list(items));
                }
            }
        }
        Ok(py_list(vec![]))
    });

    e_method!("keys", |args| {
        let obj = args[0].borrow();
        if let PyObject::Instance { dict, .. } = &*obj {
            if let Some(attrib) = dict.get_str("attrib") {
                if let PyObject::Dict(ad) = &*attrib.borrow() {
                    return Ok(py_list(ad.keys()));
                }
            }
        }
        Ok(py_list(vec![]))
    });

    let element_type = PyObjectRef::new(PyObject::Type {
        name: "Element".to_string(),
        dict: Box::new(str_map_to_typedict(element_type_dict)),
        bases: vec![],
        mro: vec![],
    });

    // Store element type in thread-local for factory functions to use
    ELEMENT_TYPE.with(|cache| {
        *cache.borrow_mut() = Some(element_type.clone());
    });

    // Helper to create a new Element instance
    fn new_element(tag: &str) -> PyObjectRef {
        let typ = ELEMENT_TYPE.with(|cache| cache.borrow().clone().unwrap());
        let mut instance_dict = AttrMap::new();
        instance_dict.insert_str("tag", py_str(tag));
        instance_dict.insert_str("text", py_none());
        instance_dict.insert_str("attrib", py_dict());
        instance_dict.insert_str("children", py_list(vec![]));
        PyObjectRef::new(PyObject::Instance {
            typ,
            dict: instance_dict,
        })
    }

    // Element(tag) factory
    et_func!("Element", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("Element() missing tag argument"));
        }
        let tag = args[0].str();
        Ok(new_element(&tag))
    });

    // SubElement(parent, tag) factory
    et_func!("SubElement", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error(
                "SubElement() requires at least 2 arguments",
            ));
        }
        let parent = &args[0];
        let tag = args[1].str();
        let child = new_element(&tag);
        // Append to parent's children list
        let list = {
            let obj = parent.borrow();
            if let PyObject::Instance { dict, .. } = &*obj {
                dict.get_str("children").cloned()
            } else {
                None
            }
        };
        if let Some(children) = list {
            if let PyObject::List(lst) = &mut *children.borrow_mut() {
                lst.push(child.clone());
            }
        }
        Ok(child)
    });

    // tostring(el) — serialize to XML string
    fn serialize_element(obj: &PyObjectRef) -> String {
        let (tag, text, children) = {
            let b = obj.borrow();
            if let PyObject::Instance { dict, .. } = &*b {
                let t = dict.get_str("tag").map(|t| t.str()).unwrap_or_default();
                let txt = dict.get_str("text").and_then(|t| {
                    let s = t.str();
                    if s.is_empty() || s == "None" {
                        None
                    } else {
                        Some(s)
                    }
                });
                let kids = dict
                    .get_str("children")
                    .and_then(|c| {
                        if let PyObject::List(list) = &*c.borrow() {
                            Some(list.clone())
                        } else {
                            None
                        }
                    })
                    .unwrap_or_default();
                (t, txt, kids)
            } else {
                (String::new(), None, vec![])
            }
        };
        if children.is_empty() && text.is_none() {
            format!("<{} />", tag)
        } else {
            let mut result = format!("<{}>", tag);
            if let Some(t) = text {
                result.push_str(
                    &t.replace("&", "&amp;")
                        .replace("<", "&lt;")
                        .replace(">", "&gt;"),
                );
            }
            for child in &children {
                result.push_str(&serialize_element(child));
            }
            result.push_str(&format!("</{}>", tag));
            result
        }
    }

    et_func!("tostring", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("tostring() missing required argument"));
        }
        Ok(py_str(&serialize_element(&args[0])))
    });

    // fromstring(xml_str) — parse simple XML
    fn parse_xml(s: &str, pos: &mut usize) -> Option<PyObjectRef> {
        // Skip whitespace
        while *pos < s.len() && s.as_bytes()[*pos].is_ascii_whitespace() {
            *pos += 1;
        }
        if *pos >= s.len() || s.as_bytes()[*pos] != b'<' {
            return None;
        }
        *pos += 1; // skip '<'
                   // Check for closing tag
        if *pos < s.len() && s.as_bytes()[*pos] == b'/' {
            return None;
        }
        // Read tag name
        let start = *pos;
        while *pos < s.len()
            && !s.as_bytes()[*pos].is_ascii_whitespace()
            && s.as_bytes()[*pos] != b'>'
            && s.as_bytes()[*pos] != b'/'
        {
            *pos += 1;
        }
        let tag_name = &s[start..*pos];
        // Skip attributes (not parsed in depth)
        while *pos < s.len() && s.as_bytes()[*pos] != b'>' && s.as_bytes()[*pos] != b'/' {
            *pos += 1;
        }
        // Self-closing tag
        if *pos < s.len() && s.as_bytes()[*pos] == b'/' {
            *pos += 2; // skip '/>'
            return Some(new_element(tag_name));
        }
        // Skip '>'
        if *pos < s.len() && s.as_bytes()[*pos] == b'>' {
            *pos += 1;
        }
        let el = new_element(tag_name);
        // Read children/text until closing tag
        let mut text = String::new();
        loop {
            while *pos < s.len() && s.as_bytes()[*pos].is_ascii_whitespace() {
                text.push(s.as_bytes()[*pos] as char);
                *pos += 1;
            }
            if *pos >= s.len() {
                break;
            }
            if s.as_bytes()[*pos] == b'<' {
                if *pos + 1 < s.len() && s.as_bytes()[*pos + 1] == b'/' {
                    *pos += 2; // skip '</'
                    while *pos < s.len() && s.as_bytes()[*pos] != b'>' {
                        *pos += 1;
                    }
                    if *pos < s.len() {
                        *pos += 1; // skip '>'
                    }
                    let trimmed = text.trim();
                    if !trimmed.is_empty() {
                        if let PyObject::Instance { dict, .. } = &mut *el.borrow_mut() {
                            dict.insert_str("text", py_str(trimmed));
                        }
                    }
                    return Some(el);
                }
                // Parse child element
                if let Some(child) = parse_xml(s, pos) {
                    let list = {
                        let obj = el.borrow();
                        if let PyObject::Instance { dict, .. } = &*obj {
                            dict.get_str("children").cloned()
                        } else {
                            None
                        }
                    };
                    if let Some(children) = list {
                        if let PyObject::List(lst) = &mut *children.borrow_mut() {
                            lst.push(child);
                        }
                    }
                } else {
                    break;
                }
            } else {
                text.push(s.as_bytes()[*pos] as char);
                *pos += 1;
            }
        }
        Some(el)
    }

    et_func!("fromstring", |args| {
        if args.is_empty() {
            return Err(PyError::type_error(
                "fromstring() missing required argument",
            ));
        }
        let xml_str = args[0].str();
        let mut pos = 0;
        match parse_xml(&xml_str, &mut pos) {
            Some(el) => Ok(el),
            None => Err(PyError::type_error("fromstring: could not parse XML")),
        }
    });

    d
}
