use crate::object::*;
use std::collections::HashMap;

pub fn create_email_mime_text_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    d.insert_str(
        "MIMEText",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "MIMEText".to_string(),
            func: |args| {
                if args.is_empty() {
                    return Err(PyError::type_error("MIMEText() missing required argument"));
                }
                let body = args[0].str();
                let subtype = if args.len() > 1 {
                    args[1].str()
                } else {
                    "plain".to_string()
                };

                let mut type_dict = HashMap::new();
                type_dict.insert_str(
                    "as_string",
                    PyObjectRef::new(PyObject::BuiltinFunction {
                        name: "as_string".to_string(),
                        func: |a| {
                            let inst = a[0].borrow();
                            if let PyObject::Instance { dict, .. } = &*inst {
                                let content = dict
                                    .get_str("_content")
                                    .map(|v| v.str())
                                    .unwrap_or_default();
                                let ct = dict
                                    .get_str("_content_type")
                                    .map(|v| v.str())
                                    .unwrap_or_default();
                                let mut result = format!("Content-Type: {}\r\n", ct);
                                result.push_str("Content-Transfer-Encoding: 7bit\r\n");
                                result.push_str("\r\n");
                                result.push_str(&content);
                                Ok(py_str(&result))
                            } else {
                                Err(PyError::type_error("MIMEText instance required"))
                            }
                        },
                    }),
                );

                let mime_type = PyObjectRef::new(PyObject::Type {
                    name: "MIMEText".to_string(),
                    dict: Box::new(str_map_to_typedict(type_dict)),
                    bases: vec![],
                    mro: vec![],
                });

                let mut instance_dict = AttrMap::new();
                instance_dict.insert_str("_content", py_str(&body));
                instance_dict.insert_str("_content_type", py_str(&format!("text/{}", subtype)));

                Ok(PyObjectRef::new(PyObject::Instance {
                    typ: mime_type,
                    dict: instance_dict,
                }))
            },
        }),
    );
    d
}
