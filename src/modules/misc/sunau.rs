use crate::object::*;
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// sunau module — AU audio file format stub
// ---------------------------------------------------------------------------
pub fn create_sunau_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! sunau_func {
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

    // Error types
    d.insert_str("Error", py_str("Error"));
    d.insert_str("Au_read", py_str("Au_read"));

    // Constants (Sun AU file format)
    d.insert_str("MAGIC", py_int(0x2e736e64)); // ".snd" magic
    d.insert_str("SND_MAGIC", py_int(0x2e736e64));
    d.insert_str("SND_HEADER_SIZE", py_int(24));

    // Encoding constants
    d.insert_str("ULAW", py_int(1));
    d.insert_str("LINEAR8", py_int(2));
    d.insert_str("LINEAR16", py_int(3));
    d.insert_str("LINEAR24", py_int(4));
    d.insert_str("LINEAR32", py_int(5));
    d.insert_str("FLOAT", py_int(6));
    d.insert_str("DOUBLE", py_int(7));
    d.insert_str("ADPCM_G721", py_int(23));
    d.insert_str("ADPCM_G722", py_int(24));
    d.insert_str("ADPCM_G723_3", py_int(25));
    d.insert_str("ADPCM_G723_5", py_int(26));
    d.insert_str("ALAW_8", py_int(27));

    // open() — returns an Au_read stub
    sunau_func!("open", |args| {
        if args.is_empty() {
            return Err(PyError::type_error(
                "open() missing required argument: file",
            ));
        }
        // Return a minimal Au_read object stub
        let mut instance_dict = AttrMap::new();
        instance_dict.insert_str("nchannels", py_int(1));
        instance_dict.insert_str("sampwidth", py_int(2));
        instance_dict.insert_str("framerate", py_int(8000));
        instance_dict.insert_str("nframes", py_int(0));
        instance_dict.insert_str("encoding", py_int(1)); // ULAW
        instance_dict.insert_str("_file", args[0].clone());

        let mut type_dict: HashMap<String, PyObjectRef> = HashMap::new();
        type_dict.insert_str(
            "getnchannels",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "getnchannels".to_string(),
                func: |self_args| {
                    if self_args.is_empty() {
                        return Err(PyError::type_error("getnchannels() missing self"));
                    }
                    let inst = self_args[0].borrow();
                    if let PyObject::Instance { dict, .. } = &*inst {
                        Ok(dict.get_str("nchannels").cloned().unwrap_or(py_int(1)))
                    } else {
                        Err(PyError::type_error("getnchannels: not an Au_read instance"))
                    }
                },
            }),
        );
        type_dict.insert_str(
            "getsampwidth",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "getsampwidth".to_string(),
                func: |self_args| {
                    if self_args.is_empty() {
                        return Err(PyError::type_error("getsampwidth() missing self"));
                    }
                    let inst = self_args[0].borrow();
                    if let PyObject::Instance { dict, .. } = &*inst {
                        Ok(dict.get_str("sampwidth").cloned().unwrap_or(py_int(2)))
                    } else {
                        Err(PyError::type_error("getsampwidth: not an Au_read instance"))
                    }
                },
            }),
        );
        type_dict.insert_str(
            "getframerate",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "getframerate".to_string(),
                func: |self_args| {
                    if self_args.is_empty() {
                        return Err(PyError::type_error("getframerate() missing self"));
                    }
                    let inst = self_args[0].borrow();
                    if let PyObject::Instance { dict, .. } = &*inst {
                        Ok(dict.get_str("framerate").cloned().unwrap_or(py_int(8000)))
                    } else {
                        Err(PyError::type_error("getframerate: not an Au_read instance"))
                    }
                },
            }),
        );
        type_dict.insert_str(
            "getnframes",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "getnframes".to_string(),
                func: |self_args| {
                    if self_args.is_empty() {
                        return Err(PyError::type_error("getnframes() missing self"));
                    }
                    let inst = self_args[0].borrow();
                    if let PyObject::Instance { dict, .. } = &*inst {
                        Ok(dict.get_str("nframes").cloned().unwrap_or(py_int(0)))
                    } else {
                        Err(PyError::type_error("getnframes: not an Au_read instance"))
                    }
                },
            }),
        );
        type_dict.insert_str(
            "getcomptype",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "getcomptype".to_string(),
                func: |_| Ok(py_str("NONE")),
            }),
        );
        type_dict.insert_str(
            "getcompname",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "getcompname".to_string(),
                func: |_| Ok(py_str("not compressed")),
            }),
        );
        type_dict.insert_str(
            "close",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "close".to_string(),
                func: |_| Ok(py_none()),
            }),
        );

        let typ = PyObjectRef::new(PyObject::Type {
            name: "Au_read".to_string(),
            dict: Box::new(str_map_to_typedict(type_dict)),
            bases: vec![],
            mro: vec![],
        });

        Ok(PyObjectRef::new(PyObject::Instance {
            typ,
            dict: instance_dict,
        }))
    });

    d
}
