use crate::object::*;
use std::collections::HashMap;

pub fn create_hashlib_extra_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! hle_func {
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

    hle_func!("md5", |args| {
        if args.len() != 1 {
            return Err(PyError::type_error("md5() takes exactly one argument"));
        }
        let data = args[0].borrow();
        let bytes = match &*data {
            PyObject::Bytes(b) => b.clone(),
            PyObject::Str(s) => s.as_bytes().to_vec(),
            _ => return Err(PyError::type_error("md5() argument must be bytes or str")),
        };
        use std::collections::hash_map::DefaultHasher;
        use std::hash::Hasher;
        let mut hasher = DefaultHasher::new();
        hasher.write(b"md5");
        hasher.write(&bytes);
        Ok(py_str(&format!("{:016x}", hasher.finish())))
    });

    hle_func!("sha1", |args| {
        if args.len() != 1 {
            return Err(PyError::type_error("sha1() takes exactly one argument"));
        }
        let data = args[0].borrow();
        let bytes = match &*data {
            PyObject::Bytes(b) => b.clone(),
            PyObject::Str(s) => s.as_bytes().to_vec(),
            _ => return Err(PyError::type_error("sha1() argument must be bytes or str")),
        };
        use std::collections::hash_map::DefaultHasher;
        use std::hash::Hasher;
        let mut hasher = DefaultHasher::new();
        hasher.write(b"sha1");
        hasher.write(&bytes);
        Ok(py_str(&format!("{:016x}", hasher.finish())))
    });

    hle_func!("sha256", |args| {
        if args.len() != 1 {
            return Err(PyError::type_error("sha256() takes exactly one argument"));
        }
        let data = args[0].borrow();
        let bytes = match &*data {
            PyObject::Bytes(b) => b.clone(),
            PyObject::Str(s) => s.as_bytes().to_vec(),
            _ => {
                return Err(PyError::type_error(
                    "sha256() argument must be bytes or str",
                ))
            }
        };
        use std::collections::hash_map::DefaultHasher;
        use std::hash::Hasher;
        let mut hasher = DefaultHasher::new();
        hasher.write(b"sha256");
        hasher.write(&bytes);
        Ok(py_str(&format!("{:016x}", hasher.finish())))
    });

    d
}
