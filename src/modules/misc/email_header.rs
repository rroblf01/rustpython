use crate::object::*;
use std::collections::HashMap;

pub fn create_email_header_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    // Header class stub — used by django.http.response
    d.insert_str(
        "Header",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "Header".to_string(),
            func: |args| {
                let text = if args.is_empty() {
                    String::new()
                } else {
                    args[0].str()
                };
                // Return a string wrapped as an object with __str__ for compatibility
                Ok(PyObjectRef::new(PyObject::Instance {
                    typ: PyObjectRef::new(PyObject::Type {
                        name: "email.header.Header".to_string(),
                        dict: Box::new(str_map_to_typedict(HashMap::new())),
                        bases: vec![],
                        mro: vec![],
                    }),
                    dict: AttrMap::from([
                        ("_text".to_string(), py_str(&text)),
                        (
                            "__str__".to_string(),
                            PyObjectRef::new(PyObject::BuiltinFunction {
                                name: "__str__".to_string(),
                                func: |a| {
                                    let inst = a[0].borrow();
                                    if let PyObject::Instance { dict, .. } = &*inst {
                                        if let Some(t) = dict.get_str("_text") {
                                            return Ok(t.clone());
                                        }
                                    }
                                    Ok(py_str(""))
                                },
                            }),
                        ),
                    ]),
                }))
            },
        }),
    );
    d
}
