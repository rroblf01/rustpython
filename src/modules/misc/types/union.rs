use crate::object::*;
use std::collections::HashMap;

thread_local! {
    static UNION_TYPE: std::cell::RefCell<Option<PyObjectRef>> = std::cell::RefCell::new(None);
}

/// `__args__` of a `types.UnionType` instance (`int | str`), if `obj` is one
/// — checked by the ad-hoc type's own NAME (`"types.UnionType"`, unique to
/// this constructor) rather than object identity, avoiding a recursive
/// `get_union_type()` call from inside `make_union`'s own flattening pass.
pub fn union_args(obj: &PyObjectRef) -> Option<Vec<PyObjectRef>> {
    if let PyObject::Instance { typ, dict } = &*obj.borrow() {
        if matches!(&*typ.borrow(), PyObject::Type { name, .. } if name == "types.UnionType") {
            if let Some(a) = dict.get("__args__") {
                if let PyObject::Tuple(items) = &*a.borrow() {
                    return Some(items.clone());
                }
            }
        }
    }
    None
}

/// Builds (or extends) a PEP 604 union (`int | str`, `int | str | None`).
/// Flattens nested unions and de-duplicates by value equality — matching
/// real CPython (`int | int == int`, `int | (str | int) == int | str`).
/// A single remaining member collapses to that member directly, not a
/// one-element union (`int | int` IS `int`, not `UnionType` wrapping it).
pub fn make_union(parts: Vec<PyObjectRef>) -> PyObjectRef {
    let mut members: Vec<PyObjectRef> = Vec::new();
    for part in parts {
        let flattened = union_args(&part).unwrap_or_else(|| vec![part]);
        for m in flattened {
            if !members
                .iter()
                .any(|existing| existing.is(&m) || existing.equals(&m).unwrap_or(false))
            {
                members.push(m);
            }
        }
    }
    if members.len() == 1 {
        return members.into_iter().next().unwrap();
    }
    let mut inst_dict = AttrMap::new();
    inst_dict.insert_str("__args__", py_tuple(members));
    PyObjectRef::new(PyObject::Instance {
        typ: get_union_type(),
        dict: inst_dict,
    })
}

fn union_member_repr(m: &PyObjectRef) -> String {
    match &*m.borrow() {
        PyObject::None => "None".to_string(),
        PyObject::Type { name, .. } => name.clone(),
        _ => m.repr(),
    }
}

fn build_union_type() -> PyObjectRef {
    let mut type_dict: HashMap<String, PyObjectRef> = HashMap::new();
    macro_rules! bf {
        ($name:expr, $f:expr) => {
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: $name.to_string(),
                func: $f,
            })
        };
    }
    type_dict.insert(
        "__repr__".to_string(),
        bf!("__repr__", |args| {
            if args.is_empty() {
                return Err(PyError::type_error("__repr__ missing argument"));
            }
            let members = union_args(&args[0]).unwrap_or_default();
            let parts: Vec<String> = members.iter().map(union_member_repr).collect();
            Ok(py_str(&parts.join(" | ")))
        }),
    );
    // Order-independent membership comparison (real CPython: `int | str ==
    // str | int`) — NOT a positional/sequence comparison.
    type_dict.insert(
        "__eq__".to_string(),
        bf!("__eq__", |args| {
            if args.len() < 2 {
                return Ok(py_not_implemented());
            }
            let a = match union_args(&args[0]) {
                Some(a) => a,
                None => return Ok(py_not_implemented()),
            };
            let b = match union_args(&args[1]) {
                Some(b) => b,
                None => return Ok(py_not_implemented()),
            };
            if a.len() != b.len() {
                return Ok(py_bool(false));
            }
            for x in &a {
                if !b.iter().any(|y| x.equals(y).unwrap_or(false)) {
                    return Ok(py_bool(false));
                }
            }
            Ok(py_bool(true))
        }),
    );
    // Order-independent hash (XOR, matching the order-independent __eq__
    // above) so a union is usable as a dict key/set member consistently
    // regardless of the order its members were written in.
    type_dict.insert(
        "__hash__".to_string(),
        bf!("__hash__", |args| {
            if args.is_empty() {
                return Err(PyError::type_error("__hash__ missing argument"));
            }
            let members = union_args(&args[0]).unwrap_or_default();
            let mut h: i64 = 0;
            for m in &members {
                h ^= m.hash()? as i64;
            }
            Ok(py_int(h))
        }),
    );
    type_dict.insert(
        "__or__".to_string(),
        bf!("__or__", |args| {
            if args.len() < 2 {
                return Err(PyError::type_error("__or__() missing argument"));
            }
            Ok(make_union(vec![args[0].clone(), args[1].clone()]))
        }),
    );
    type_dict.insert(
        "__ror__".to_string(),
        bf!("__ror__", |args| {
            if args.len() < 2 {
                return Err(PyError::type_error("__ror__() missing argument"));
            }
            Ok(make_union(vec![args[1].clone(), args[0].clone()]))
        }),
    );
    PyObjectRef::new(PyObject::Type {
        name: "types.UnionType".to_string(),
        dict: Box::new(str_map_to_typedict(type_dict)),
        bases: vec![],
        mro: vec![],
    })
}

pub fn get_union_type() -> PyObjectRef {
    let existing = UNION_TYPE.with(|c| c.borrow().clone());
    if let Some(t) = existing {
        return t;
    }
    let typ = build_union_type();
    UNION_TYPE.with(|c| {
        *c.borrow_mut() = Some(typ.clone());
    });
    typ
}
