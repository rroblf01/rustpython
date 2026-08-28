// Split from src/object/pydict.rs — dict view helpers (dict_keys/values/items).
use super::*;
use crate::object::*;

/// Build a live dict-view instance (`dict_keys` / `dict_values` /
/// `dict_items`) over `d`. Views expose iteration, len, contains, `.mapping`,
/// equality against sets/lists, and — for keys/items — the full set
/// operators (`|`, `&`, `-`, `^`), returning plain sets like CPython.
pub fn make_dict_view(kind: &str, d: PyObjectRef) -> crate::object::PyObjectRef {
    use std::cell::RefCell;
    use std::collections::HashMap as StdHashMap;
    thread_local! {
        static VIEW_TYPES: RefCell<StdHashMap<String, crate::object::PyObjectRef>> =
            RefCell::new(StdHashMap::new());
    }
    use crate::object::{
        py_bool, py_int, py_str, builtin_iter, AttrMap, ObjectAccess, PyError, PyObject,
        PyObjectRef, PySet,
    };

    let is_items = kind == "dict_items";
    let is_values = kind == "dict_values";

    fn view_kind(view: &PyObjectRef) -> String {
        match view.borrow().get_attribute("kind_name") {
            Ok(k) => k.borrow().str(),
            Err(_) => String::new(),
        }
    }

    fn view_elems(view: &PyObjectRef) -> Vec<PyObjectRef> {
        let mapping = match view.borrow().get_attribute("mapping") {
            Ok(m) => m,
            Err(_) => return Vec::new(),
        };
        let items = match &*mapping.borrow() {
            PyObject::Dict(dd) => dd.items(),
            _ => return Vec::new(),
        };
        let tn = view_kind(view);
        if tn == "items" {
            items
                .into_iter()
                .map(|(k, v)| py_tuple(vec![k, v]))
                .collect()
        } else if tn == "values" {
            items.into_iter().map(|(_, v)| v).collect()
        } else {
            items.into_iter().map(|(k, _)| k).collect()
        }
    }

    fn elems_of(other: &PyObjectRef) -> Option<Vec<PyObjectRef>> {
        match &*other.borrow() {
            PyObject::Set(s2) | PyObject::FrozenSet(s2) => Some(s2.to_vec()),
            PyObject::List(l2) => Some(l2.clone()),
            PyObject::Tuple(t2) => Some(t2.clone()),
            _ => {
                let is_view = other.borrow().get_attribute("kind_name").is_ok();
                if is_view {
                    Some(view_elems(other))
                } else {
                    None
                }
            }
        }
    }

    fn lin_contains(hay: &[PyObjectRef], needle: &PyObjectRef) -> PyResult<bool> {
        for h in hay {
            if h.is(needle) || h.equals(needle)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn view_setop(
        args: &[PyObjectRef],
        f: impl Fn(&Vec<PyObjectRef>, &Vec<PyObjectRef>, &dyn Fn(&PyObjectRef, &Vec<PyObjectRef>) -> PyResult<bool>) -> PyResult<Vec<PyObjectRef>>,
    ) -> PyResult<PyObjectRef> {
        let a = view_elems(&args[0]);
        let b = elems_of(&args[1]).ok_or_else(|| {
            PyError::type_error("unsupported operand type(s) for set operation with dictview")
        })?;
        let pred = |needle: &PyObjectRef, hay: &Vec<PyObjectRef>| lin_contains(hay, needle);
        let out = f(&a, &b, &pred)?;
        let mut s = PySet::new();
        for e in out {
            s.add(e)?;
        }
        Ok(PyObjectRef::new(PyObject::Set(s)))
    }

    let typ = VIEW_TYPES.with(|c| {
        if let Some(t) = c.borrow().get(kind) {
            return t.clone();
        }
        let bf = |name: &'static str, f: BuiltinFunc| {
            PyObjectRef::new(PyObject::BuiltinFunction { name: name.to_string(), func: f })
        };
        let mut td: StdHashMap<String, PyObjectRef> = StdHashMap::new();

        td.insert("__iter__".into(), bf("__iter__", |args| {
            let kind = view_kind(&args[0]);
            let mapping = match args[0].borrow().get_attribute("mapping") {
                Ok(m) => m,
                Err(_) => return builtin_iter(&[py_list(view_elems(&args[0]))]),
            };
            if let PyObject::Dict(d) = &*mapping.borrow() {
                let version = d.version();
                if kind == "keys" {
                    return Ok(PyObjectRef::new(PyObject::DictIter {
                        dict: mapping.clone(),
                        keys: d.keys(),
                        index: 0,
                        expected_version: version,
                    }));
                } else if kind == "values" {
                    return Ok(PyObjectRef::new(PyObject::DictValuesIter {
                        dict: mapping.clone(),
                        values: d.values(),
                        index: 0,
                        expected_version: version,
                    }));
                } else if kind == "items" {
                    return Ok(PyObjectRef::new(PyObject::DictItemsIter {
                        dict: mapping.clone(),
                        items: d.items(),
                        index: 0,
                        expected_version: version,
                    }));
                }
            }
            builtin_iter(&[py_list(view_elems(&args[0]))])
        }));
        td.insert("__reversed__".into(), bf("__reversed__", |args| {
            let kind = view_kind(&args[0]);
            let mapping = match args[0].borrow().get_attribute("mapping") {
                Ok(m) => m,
                Err(_) => {
                    let v = view_elems(&args[0]);
                    let mut rev = v.clone();
                    rev.reverse();
                    return Ok(PyObjectRef::new(PyObject::ListIter { list: rev, index: 0 }));
                }
            };
            if let PyObject::Dict(d) = &*mapping.borrow() {
                let version = d.version();
                if kind == "keys" {
                    let keys = d.keys();
                    let idx = if keys.is_empty() { -1 } else { (keys.len() as isize) - 1 };
                    return Ok(PyObjectRef::new(PyObject::DictRevIter { dict: mapping.clone(), keys, index: idx, expected_version: version }));
                } else if kind == "values" {
                    let vals = d.values();
                    let mut rev = vals.clone();
                    rev.reverse();
                    // values reversed – use ListIter with version check via DictValuesIter reversed?
                    // For simplicity use ListIter but with version check via extra wrapper – use DictValuesIter reversed logic via ListIter + manual check?
                    // We'll just use a DictValuesIter reversed by storing reversed values
                    let version = d.version();
                    let mut rev_vals = d.values();
                    rev_vals.reverse();
                    // Reuse DictValuesIter but with reversed order and index 0
                    // Need to handle version check – create a DictValuesIter with reversed list
                    return Ok(PyObjectRef::new(PyObject::DictValuesIter { dict: mapping.clone(), values: rev_vals, index: 0, expected_version: version }));
                } else if kind == "items" {
                    let mut items = d.items();
                    items.reverse();
                    let version = d.version();
                    return Ok(PyObjectRef::new(PyObject::DictItemsIter { dict: mapping.clone(), items, index: 0, expected_version: version }));
                }
            }
            let v = view_elems(&args[0]);
            let mut rev = v.clone();
            rev.reverse();
            Ok(PyObjectRef::new(PyObject::ListIter { list: rev, index: 0 }))
        }));
        td.insert("__len__".into(), bf("__len__", |args| {
            Ok(py_int(view_elems(&args[0]).len() as i64))
        }));
        td.insert("__contains__".into(), bf("__contains__", |args| {
            let elems = view_elems(&args[0]);
            lin_contains(&elems, &args[1]).map(py_bool)
        }));
        td.insert("__repr__".into(), bf("__repr__", |args| {
            let tn = format!("dict_{}", view_kind(&args[0]));
            let inner: Vec<String> =
                view_elems(&args[0]).into_iter().map(|e| e.repr()).collect();
            Ok(py_str(&format!("{}([{}])", tn, inner.join(", "))))
        }));
        td.insert("__eq__".into(), bf("__eq__", |args| {
            let mine = view_elems(&args[0]);
            match elems_of(&args[1]) {
                Some(theirs) => {
                    if mine.len() != theirs.len() {
                        return Ok(py_bool(false));
                    }
                    for m in &mine {
                        if !lin_contains(&theirs, m)? {
                            return Ok(py_bool(false));
                        }
                    }
                    Ok(py_bool(true))
                }
                None => Ok(py_bool(false)),
            }
        }));
        td.insert("__or__".into(), bf("__or__", |args| {
            view_setop(args, |a, b, has| {
                let mut out = a.clone();
                for e in b {
                    if !has(e, a)? {
                        out.push(e.clone());
                    }
                }
                Ok(out)
            })
        }));
        td.insert("__and__".into(), bf("__and__", |args| {
            view_setop(args, |a, b, has| {
                Ok(a.iter().filter(|e| has(e, b).unwrap_or(false)).cloned().collect())
            })
        }));
        td.insert("__sub__".into(), bf("__sub__", |args| {
            view_setop(args, |a, b, has| {
                Ok(a.iter().filter(|e| !has(e, b).unwrap_or(false)).cloned().collect())
            })
        }));
        td.insert("__xor__".into(), bf("__xor__", |args| {
            view_setop(args, |a, b, has| {
                let mut out: Vec<PyObjectRef> =
                    a.iter().filter(|e| !has(e, b).unwrap_or(false)).cloned().collect();
                for e in b {
                    if !has(e, a)? {
                        out.push(e.clone());
                    }
                }
                Ok(out)
            })
        }));
        td.insert("__le__".into(), bf("__le__", |args| {
            let mine = view_elems(&args[0]);
            let theirs = match elems_of(&args[1]) {
                Some(v) => v,
                None => return Ok(py_not_implemented()),
            };
            if mine.len() > theirs.len() {
                return Ok(py_bool(false));
            }
            for m in &mine {
                if !lin_contains(&theirs, m)? {
                    return Ok(py_bool(false));
                }
            }
            Ok(py_bool(true))
        }));
        td.insert("__lt__".into(), bf("__lt__", |args| {
            let mine = view_elems(&args[0]);
            let theirs = match elems_of(&args[1]) {
                Some(v) => v,
                None => return Ok(py_not_implemented()),
            };
            if mine.len() >= theirs.len() {
                return Ok(py_bool(false));
            }
            for m in &mine {
                if !lin_contains(&theirs, m)? {
                    return Ok(py_bool(false));
                }
            }
            Ok(py_bool(true))
        }));
        td.insert("__ge__".into(), bf("__ge__", |args| {
            let mine = view_elems(&args[0]);
            let theirs = match elems_of(&args[1]) {
                Some(v) => v,
                None => return Ok(py_not_implemented()),
            };
            if mine.len() < theirs.len() {
                return Ok(py_bool(false));
            }
            for t in &theirs {
                if !lin_contains(&mine, t)? {
                    return Ok(py_bool(false));
                }
            }
            Ok(py_bool(true))
        }));
        td.insert("__gt__".into(), bf("__gt__", |args| {
            let mine = view_elems(&args[0]);
            let theirs = match elems_of(&args[1]) {
                Some(v) => v,
                None => return Ok(py_not_implemented()),
            };
            if mine.len() <= theirs.len() {
                return Ok(py_bool(false));
            }
            for t in &theirs {
                if !lin_contains(&mine, t)? {
                    return Ok(py_bool(false));
                }
            }
            Ok(py_bool(true))
        }));
        // reflected set ops for `set | dict_keys` etc.
        td.insert("__ror__".into(), bf("__ror__", |args| {
            view_setop(&[args[1].clone(), args[0].clone()], |a, b, has| {
                let mut out = b.clone();
                for e in a {
                    if !has(e, b)? {
                        out.push(e.clone());
                    }
                }
                Ok(out)
            })
        }));
        td.insert("__rand__".into(), bf("__rand__", |args| {
            view_setop(&[args[1].clone(), args[0].clone()], |a, b, has| {
                Ok(b.iter().filter(|e| has(e, a).unwrap_or(false)).cloned().collect())
            })
        }));
        td.insert("__rsub__".into(), bf("__rsub__", |args| {
            view_setop(&[args[1].clone(), args[0].clone()], |a, b, has| {
                Ok(a.iter().filter(|e| !has(e, b).unwrap_or(false)).cloned().collect())
            })
        }));
        td.insert("__rxor__".into(), bf("__rxor__", |args| {
            view_setop(&[args[1].clone(), args[0].clone()], |a, b, has| {
                let mut out: Vec<PyObjectRef> =
                    b.iter().filter(|e| !has(e, a).unwrap_or(false)).cloned().collect();
                for e in a {
                    if !has(e, b)? {
                        out.push(e.clone());
                    }
                }
                Ok(out)
            })
        }));

        let t = PyObjectRef::new(PyObject::Type {
            name: kind.to_string(),
            dict: Box::new(crate::object::str_map_to_typedict(td)),
            bases: vec![],
            mro: vec![],
        });
        c.borrow_mut().insert(kind.to_string(), t.clone());
        t
    });

    let mut attrs = AttrMap::new();
    attrs.insert_str("mapping", d);
    let kn = if is_items { "items" } else if is_values { "values" } else { "keys" };
    attrs.insert_str("kind_name", py_str(kn));
    PyObjectRef::new(PyObject::Instance { typ, dict: attrs })
}
