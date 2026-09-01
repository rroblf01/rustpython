use crate::object::*;
use std::collections::HashMap;

pub fn create_configparser_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();

    // Helper: parse INI string into sections
    fn parse_ini_string(data: &str) -> HashMap<String, HashMap<String, String>> {
        let mut sections: HashMap<String, HashMap<String, String>> = HashMap::new();
        let mut current_section: Option<String> = None;

        // Start with a pseudo-section for DEFAULT values
        sections.insert("DEFAULT".to_string(), HashMap::new());

        for line in data.lines() {
            let trimmed = line.trim();
            // Skip empty lines and comments
            if trimmed.is_empty() || trimmed.starts_with(';') || trimmed.starts_with('#') {
                continue;
            }

            // Section header: [sectionname]
            if trimmed.starts_with('[') {
                if let Some(end) = trimmed.find(']') {
                    let name = trimmed[1..end].trim().to_string();
                    if !name.is_empty() {
                        current_section = Some(name.clone());
                        sections.entry(name).or_insert_with(HashMap::new);
                    }
                }
                continue;
            }

            // Key = value (or key: value)
            if let Some(eq_pos) = trimmed.find('=').or_else(|| trimmed.find(':')) {
                let key = trimmed[..eq_pos].trim().to_string();
                let value = trimmed[eq_pos + 1..].trim().to_string();
                if !key.is_empty() {
                    let section_name = current_section
                        .clone()
                        .unwrap_or_else(|| "DEFAULT".to_string());
                    let section = sections.entry(section_name).or_insert_with(HashMap::new);
                    section.insert(key, value);
                }
            }
        }

        sections
    }

    // ConfigParser class — constructor
    d.insert_str(
        "ConfigParser",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "ConfigParser".to_string(),
            func: |_args| {
                let mut type_dict = HashMap::new();

                // read_string(self, string) — parse INI from a string
                type_dict.insert_str(
                    "read_string",
                    PyObjectRef::new(PyObject::BuiltinFunction {
                        name: "read_string".to_string(),
                        func: |inner_args| {
                            if inner_args.len() < 2 {
                                return Err(PyError::type_error(
                                    "read_string() missing required argument: string",
                                ));
                            }
                            let data = inner_args[1].str();

                            let sections_ref = {
                                let inst = inner_args[0].borrow();
                                if let PyObject::Instance { dict, .. } = &*inst {
                                    dict.get_str("_sections").cloned().unwrap_or(py_dict())
                                } else {
                                    return Err(PyError::type_error(
                                        "read_string(): not a ConfigParser instance",
                                    ));
                                }
                            };

                            let parsed = parse_ini_string(&data);
                            // Merge parsed sections into existing sections
                            if let PyObject::Dict(ref mut sections_dict) =
                                &mut *sections_ref.borrow_mut()
                            {
                                for (section_name, options) in parsed {
                                    let section_key = py_str(&section_name);
                                    // Try to get existing section dict
                                    let existing =
                                        sections_dict.get(&section_key).ok().and_then(|o| o);
                                    if let Some(existing_ref) = existing {
                                        if let PyObject::Dict(ref mut existing_dict) =
                                            &mut *existing_ref.borrow_mut()
                                        {
                                            for (key, val) in options {
                                                let _ =
                                                    existing_dict.set(py_str(&key), py_str(&val));
                                            }
                                        }
                                    } else {
                                        // Create new section dict
                                        let option_dict = py_dict();
                                        if let PyObject::Dict(ref mut od) =
                                            &mut *option_dict.borrow_mut()
                                        {
                                            for (key, val) in options {
                                                let _ = od.set(py_str(&key), py_str(&val));
                                            }
                                        }
                                        let _ =
                                            sections_dict.set(py_str(&section_name), option_dict);
                                    }
                                }
                            }

                            Ok(py_none())
                        },
                    }),
                );

                // read(self, filename) — parse INI from a file
                type_dict.insert_str(
                    "read",
                    PyObjectRef::new(PyObject::BuiltinFunction {
                        name: "read".to_string(),
                        func: |inner_args| {
                            if inner_args.len() < 2 {
                                return Err(PyError::type_error(
                                    "read() missing required argument: filename",
                                ));
                            }
                            let filename = inner_args[1].str();
                            let content = match std::fs::read_to_string(&filename) {
                                Ok(s) => s,
                                Err(e) => {
                                    return Err(PyError::type_error(format!(
                                        "Cannot read file '{}': {}",
                                        filename, e
                                    )))
                                }
                            };

                            // Reuse read_string logic — call it on self
                            let sections_ref = {
                                let inst = inner_args[0].borrow();
                                if let PyObject::Instance { dict, .. } = &*inst {
                                    dict.get_str("_sections").cloned().unwrap_or(py_dict())
                                } else {
                                    return Err(PyError::type_error(
                                        "read(): not a ConfigParser instance",
                                    ));
                                }
                            };

                            let parsed = parse_ini_string(&content);
                            if let PyObject::Dict(ref mut sections_dict) =
                                &mut *sections_ref.borrow_mut()
                            {
                                for (section_name, options) in parsed {
                                    let section_key = py_str(&section_name);
                                    let existing =
                                        sections_dict.get(&section_key).ok().and_then(|o| o);
                                    if let Some(existing_ref) = existing {
                                        if let PyObject::Dict(ref mut existing_dict) =
                                            &mut *existing_ref.borrow_mut()
                                        {
                                            for (key, val) in options {
                                                let _ =
                                                    existing_dict.set(py_str(&key), py_str(&val));
                                            }
                                        }
                                    } else {
                                        let option_dict = py_dict();
                                        if let PyObject::Dict(ref mut od) =
                                            &mut *option_dict.borrow_mut()
                                        {
                                            for (key, val) in options {
                                                let _ = od.set(py_str(&key), py_str(&val));
                                            }
                                        }
                                        let _ =
                                            sections_dict.set(py_str(&section_name), option_dict);
                                    }
                                }
                            }

                            // Return list of successfully read files
                            Ok(py_list(vec![inner_args[1].clone()]))
                        },
                    }),
                );

                // sections(self) — return list of section names
                type_dict.insert_str(
                    "sections",
                    PyObjectRef::new(PyObject::BuiltinFunction {
                        name: "sections".to_string(),
                        func: |inner_args| {
                            if inner_args.is_empty() {
                                return Err(PyError::type_error(
                                    "sections() missing self argument",
                                ));
                            }
                            let inst = inner_args[0].borrow();
                            if let PyObject::Instance { dict, .. } = &*inst {
                                let sections_ref =
                                    dict.get_str("_sections").cloned().unwrap_or(py_dict());
                                let sections_borrow = sections_ref.borrow();
                                if let PyObject::Dict(sections_dict) = &*sections_borrow {
                                    let mut names: Vec<PyObjectRef> = Vec::new();
                                    for (k, _) in sections_dict.items() {
                                        let name = k.str();
                                        if name != "DEFAULT" {
                                            names.push(py_str(&name));
                                        }
                                    }
                                    Ok(py_list(names))
                                } else {
                                    Ok(py_list(vec![]))
                                }
                            } else {
                                Err(PyError::type_error(
                                    "sections(): not a ConfigParser instance",
                                ))
                            }
                        },
                    }),
                );

                // options(self, section) — return list of option names in a section
                type_dict.insert_str(
                    "options",
                    PyObjectRef::new(PyObject::BuiltinFunction {
                        name: "options".to_string(),
                        func: |inner_args| {
                            if inner_args.len() < 2 {
                                return Err(PyError::type_error(
                                    "options() missing required argument: section",
                                ));
                            }
                            let section_name = inner_args[1].str();
                            let inst = inner_args[0].borrow();
                            if let PyObject::Instance { dict, .. } = &*inst {
                                let sections_ref =
                                    dict.get_str("_sections").cloned().unwrap_or(py_dict());
                                drop(inst);
                                let sections_borrow = sections_ref.borrow();
                                if let PyObject::Dict(sections_dict) = &*sections_borrow {
                                    let section_key = py_str(&section_name);
                                    if let Ok(Some(section_ref)) = sections_dict.get(&section_key) {
                                        let section_borrow = section_ref.borrow();
                                        if let PyObject::Dict(option_dict) = &*section_borrow {
                                            let mut keys: Vec<PyObjectRef> = option_dict
                                                .keys()
                                                .into_iter()
                                                .map(|k| py_str(&k.str()))
                                                .collect();
                                            // Also include DEFAULT options
                                            if section_name != "DEFAULT" {
                                                if let Ok(Some(default_ref)) =
                                                    sections_dict.get(&py_str("DEFAULT"))
                                                {
                                                    if let PyObject::Dict(default_dict) =
                                                        &*default_ref.borrow()
                                                    {
                                                        for k in default_dict.keys() {
                                                            let kstr = k.str();
                                                            if !keys
                                                                .iter()
                                                                .any(|k2| k2.str() == kstr)
                                                            {
                                                                keys.push(py_str(&kstr));
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                            Ok(py_list(keys))
                                        } else {
                                            Ok(py_list(vec![]))
                                        }
                                    } else {
                                        Err(PyError::type_error(format!(
                                            "No section '{}'",
                                            section_name
                                        )))
                                    }
                                } else {
                                    Ok(py_list(vec![]))
                                }
                            } else {
                                Err(PyError::type_error(
                                    "options(): not a ConfigParser instance",
                                ))
                            }
                        },
                    }),
                );

                // get(self, section, option, fallback=None) — get a value
                type_dict.insert_str(
                    "get",
                    PyObjectRef::new(PyObject::BuiltinFunction {
                        name: "get".to_string(),
                        func: |inner_args| {
                            if inner_args.len() < 3 {
                                return Err(PyError::type_error(
                                    "get() missing required arguments: section, option",
                                ));
                            }
                            let section_name = inner_args[1].str();
                            let option_name = inner_args[2].str();
                            let fallback = if inner_args.len() > 3 {
                                Some(inner_args[3].clone())
                            } else {
                                None
                            };

                            let inst = inner_args[0].borrow();
                            if let PyObject::Instance { dict, .. } = &*inst {
                                let sections_ref =
                                    dict.get_str("_sections").cloned().unwrap_or(py_dict());
                                drop(inst);

                                let sections_borrowed = sections_ref.borrow();
                                if let PyObject::Dict(sections_dict) = &*sections_borrowed {
                                    // Try the specified section
                                    let section_key = py_str(&section_name);
                                    if let Ok(Some(section_ref)) = sections_dict.get(&section_key) {
                                        if let PyObject::Dict(option_dict) = &*section_ref.borrow()
                                        {
                                            let option_key = py_str(&option_name);
                                            if let Ok(Some(val)) = option_dict.get(&option_key) {
                                                return Ok(val);
                                            }
                                        }
                                    }
                                    // Try DEFAULT section
                                    if section_name != "DEFAULT" {
                                        if let Ok(Some(default_ref)) =
                                            sections_dict.get(&py_str("DEFAULT"))
                                        {
                                            if let PyObject::Dict(default_dict) =
                                                &*default_ref.borrow()
                                            {
                                                let option_key = py_str(&option_name);
                                                if let Ok(Some(val)) = default_dict.get(&option_key)
                                                {
                                                    return Ok(val);
                                                }
                                            }
                                        }
                                    }
                                }
                            }

                            // Return fallback or raise error
                            match fallback {
                                Some(fb) => Ok(fb),
                                None => Err(PyError::type_error(format!(
                                    "No option '{}' in section '{}'",
                                    option_name, section_name
                                ))),
                            }
                        },
                    }),
                );

                // items(self, section) — return list of (option, value) tuples
                type_dict.insert_str(
                    "items",
                    PyObjectRef::new(PyObject::BuiltinFunction {
                        name: "items".to_string(),
                        func: |inner_args| {
                            if inner_args.len() < 2 {
                                return Err(PyError::type_error(
                                    "items() missing required argument: section",
                                ));
                            }
                            let section_name = inner_args[1].str();
                            let inst = inner_args[0].borrow();
                            if let PyObject::Instance { dict, .. } = &*inst {
                                let sections_ref =
                                    dict.get_str("_sections").cloned().unwrap_or(py_dict());
                                drop(inst);
                                let sections_borrow = sections_ref.borrow();
                                if let PyObject::Dict(sections_dict) = &*sections_borrow {
                                    let section_key = py_str(&section_name);
                                    if let Ok(Some(section_ref)) = sections_dict.get(&section_key) {
                                        let section_borrow = section_ref.borrow();
                                        if let PyObject::Dict(option_dict) = &*section_borrow {
                                            let mut result: Vec<PyObjectRef> = Vec::new();
                                            // Include DEFAULT options first
                                            if section_name != "DEFAULT" {
                                                if let Ok(Some(default_ref)) =
                                                    sections_dict.get(&py_str("DEFAULT"))
                                                {
                                                    if let PyObject::Dict(default_dict) =
                                                        &*default_ref.borrow()
                                                    {
                                                        for (k, v) in default_dict.items() {
                                                            result.push(py_tuple(vec![k, v]));
                                                        }
                                                    }
                                                }
                                            }
                                            // Add section-specific options
                                            for (k, v) in option_dict.items() {
                                                let kstr = k.str();
                                                // Override DEFAULT if present
                                                if let Some(pos) = result.iter().position(|t| {
                                                    if let PyObject::Tuple(items) = &*t.borrow() {
                                                        items[0].str() == kstr
                                                    } else {
                                                        false
                                                    }
                                                }) {
                                                    result[pos] = py_tuple(vec![k, v]);
                                                } else {
                                                    result.push(py_tuple(vec![k, v]));
                                                }
                                            }
                                            Ok(py_list(result))
                                        } else {
                                            Ok(py_list(vec![]))
                                        }
                                    } else {
                                        Err(PyError::type_error(format!(
                                            "No section '{}'",
                                            section_name
                                        )))
                                    }
                                } else {
                                    Ok(py_list(vec![]))
                                }
                            } else {
                                Err(PyError::type_error("items(): not a ConfigParser instance"))
                            }
                        },
                    }),
                );

                // add_section(self, name) — add a new section
                type_dict.insert_str(
                    "add_section",
                    PyObjectRef::new(PyObject::BuiltinFunction {
                        name: "add_section".to_string(),
                        func: |inner_args| {
                            if inner_args.len() < 2 {
                                return Err(PyError::type_error(
                                    "add_section() missing required argument: name",
                                ));
                            }
                            let section_name = inner_args[1].str();

                            let sections_ref = {
                                let inst = inner_args[0].borrow();
                                if let PyObject::Instance { dict, .. } = &*inst {
                                    dict.get_str("_sections").cloned().unwrap_or(py_dict())
                                } else {
                                    return Err(PyError::type_error(
                                        "add_section(): not a ConfigParser instance",
                                    ));
                                }
                            };

                            if let PyObject::Dict(ref mut sections_dict) =
                                &mut *sections_ref.borrow_mut()
                            {
                                let section_key = py_str(&section_name);
                                if sections_dict.contains(&section_key).unwrap_or(false) {
                                    return Err(PyError::type_error(format!(
                                        "Section '{}' already exists",
                                        section_name
                                    )));
                                }
                                let _ = sections_dict.set(py_str(&section_name), py_dict());
                            }

                            Ok(py_none())
                        },
                    }),
                );

                // set(self, section, option, value) — set an option
                type_dict.insert_str(
                    "set",
                    PyObjectRef::new(PyObject::BuiltinFunction {
                        name: "set".to_string(),
                        func: |inner_args| {
                            if inner_args.len() < 4 {
                                return Err(PyError::type_error(
                                    "set() missing required arguments: section, option, value",
                                ));
                            }
                            let section_name = inner_args[1].str();
                            let option_name = inner_args[2].str();
                            let value = inner_args[3].str();

                            let sections_ref = {
                                let inst = inner_args[0].borrow();
                                if let PyObject::Instance { dict, .. } = &*inst {
                                    dict.get_str("_sections").cloned().unwrap_or(py_dict())
                                } else {
                                    return Err(PyError::type_error(
                                        "set(): not a ConfigParser instance",
                                    ));
                                }
                            };

                            if let PyObject::Dict(ref mut sections_dict) =
                                &mut *sections_ref.borrow_mut()
                            {
                                let section_key = py_str(&section_name);
                                // Check section exists
                                if !sections_dict.contains(&section_key).unwrap_or(false) {
                                    return Err(PyError::type_error(format!(
                                        "No section '{}'",
                                        section_name
                                    )));
                                }
                                if let Ok(Some(existing_ref)) = sections_dict.get(&section_key) {
                                    if let PyObject::Dict(ref mut option_dict) =
                                        &mut *existing_ref.borrow_mut()
                                    {
                                        let _ =
                                            option_dict.set(py_str(&option_name), py_str(&value));
                                    }
                                }
                            }

                            Ok(py_none())
                        },
                    }),
                );

                // has_section(self, name) — check if section exists
                type_dict.insert_str(
                    "has_section",
                    PyObjectRef::new(PyObject::BuiltinFunction {
                        name: "has_section".to_string(),
                        func: |inner_args| {
                            if inner_args.len() < 2 {
                                return Err(PyError::type_error(
                                    "has_section() missing required argument: name",
                                ));
                            }
                            let section_name = inner_args[1].str();
                            let inst = inner_args[0].borrow();
                            if let PyObject::Instance { dict, .. } = &*inst {
                                let sections_ref =
                                    dict.get_str("_sections").cloned().unwrap_or(py_dict());
                                drop(inst);
                                let sections_borrow = sections_ref.borrow();
                                if let PyObject::Dict(sections_dict) = &*sections_borrow {
                                    let section_key = py_str(&section_name);
                                    let found =
                                        sections_dict.contains(&section_key).unwrap_or(false);
                                    Ok(py_bool(found))
                                } else {
                                    Ok(py_bool(false))
                                }
                            } else {
                                Err(PyError::type_error(
                                    "has_section(): not a ConfigParser instance",
                                ))
                            }
                        },
                    }),
                );

                let typ = PyObjectRef::new(PyObject::Type {
                    name: "ConfigParser".to_string(),
                    dict: Box::new(str_map_to_typedict(type_dict)),
                    bases: vec![],
                    mro: vec![],
                });

                let mut instance_dict = AttrMap::new();
                instance_dict.insert_str("_sections", py_dict());

                Ok(PyObjectRef::new(PyObject::Instance {
                    typ,
                    dict: instance_dict,
                }))
            },
        }),
    );

    d
}
