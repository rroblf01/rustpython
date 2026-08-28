use crate::object::*;
use std::collections::HashMap;
use super::helpers::EPOCH_ORDINAL;
use super::tzif::{load_tz, tz_offset_for_instant};

pub(crate) fn inst_get(obj: &PyObjectRef, name: &str) -> Option<PyObjectRef> {
    if let PyObject::Instance { dict, .. } = &*obj.borrow() {
        dict.get(name).cloned()
    } else {
        None
    }
}

pub(crate) fn inst_get_i64(obj: &PyObjectRef, name: &str) -> i64 {
    inst_get(obj, name).and_then(|v| v.as_i64()).unwrap_or(0)
}

pub(crate) fn instance_type_name(obj: &PyObjectRef) -> String {
    if let PyObject::Instance { typ, .. } = &*obj.borrow() {
        if let PyObject::Type { name, .. } = &*typ.borrow() {
            return name.clone();
        }
    }
    String::new()
}

/// UTC offset in seconds for `tzinfo`, evaluated at the naive wall-clock
/// instant given by `ordinal`/`day_seconds` (days since 0001-01-01 and
/// seconds since local midnight). Only understands this module's own
/// `timezone` and `zoneinfo.ZoneInfo` — see module-level doc comment.
pub(crate) fn get_utcoffset_seconds(tzinfo: &PyObjectRef, ordinal: i64, day_seconds: i64) -> Option<i64> {
    if matches!(tzinfo, PyObjectRef::None) {
        return None;
    }
    match instance_type_name(tzinfo).as_str() {
        "timezone" => Some(inst_get_i64(tzinfo, "_offset_seconds")),
        "ZoneInfo" => {
            let key = inst_get(tzinfo, "key").map(|v| v.str()).unwrap_or_default();
            let tz = load_tz(&key)?;
            let unix_instant = (ordinal - EPOCH_ORDINAL) * 86400 + day_seconds;
            Some(tz_offset_for_instant(&tz, unix_instant).0 as i64)
        }
        _ => None,
    }
}

pub(crate) fn tzname_for(tzinfo: &PyObjectRef, ordinal: i64, day_seconds: i64) -> Option<String> {
    if matches!(tzinfo, PyObjectRef::None) {
        return None;
    }
    match instance_type_name(tzinfo).as_str() {
        "timezone" => {
            let name = inst_get(tzinfo, "_name");
            match name {
                Some(n) if !matches!(n, PyObjectRef::None) => Some(n.str()),
                _ => {
                    let off = inst_get_i64(tzinfo, "_offset_seconds");
                    Some(format_utc_offset_name(off))
                }
            }
        }
        "ZoneInfo" => {
            let key = inst_get(tzinfo, "key").map(|v| v.str()).unwrap_or_default();
            let tz = load_tz(&key)?;
            let unix_instant = (ordinal - EPOCH_ORDINAL) * 86400 + day_seconds;
            Some(tz_offset_for_instant(&tz, unix_instant).2)
        }
        _ => None,
    }
}

pub(crate) fn format_utc_offset_name(offset_seconds: i64) -> String {
    if offset_seconds == 0 {
        return "UTC".to_string();
    }
    let sign = if offset_seconds < 0 { '-' } else { '+' };
    let abs = offset_seconds.abs();
    let h = abs / 3600;
    let m = (abs % 3600) / 60;
    if m == 0 {
        format!("UTC{}{:02}", sign, h)
    } else {
        format!("UTC{}{:02}:{:02}", sign, h, m)
    }
}

pub(crate) fn format_offset_iso(offset_seconds: i64) -> String {
    let sign = if offset_seconds < 0 { '-' } else { '+' };
    let abs = offset_seconds.abs();
    let h = abs / 3600;
    let m = (abs % 3600) / 60;
    let s = abs % 60;
    if s == 0 {
        format!("{}{:02}:{:02}", sign, h, m)
    } else {
        format!("{}{:02}:{:02}:{:02}", sign, h, m, s)
    }
}

// ---- Constructor-argument parsing (positional args + trailing kwargs dict) ----

pub(crate) struct CtorArgs {
    pub(crate) pos: Vec<PyObjectRef>,
    pub(crate) kw: HashMap<String, PyObjectRef>,
}

impl CtorArgs {
    /// `args` excludes the leading `self`/instance argument.
    pub(crate) fn parse(args: &[PyObjectRef]) -> Self {
        let mut pos = args.to_vec();
        let mut kw = HashMap::new();
        if let Some(last) = pos.last().cloned() {
            if matches!(&*last.borrow(), PyObject::Dict(_)) {
                pos.pop();
                if let PyObject::Dict(d) = &*last.borrow() {
                    for (k, v) in d.items() {
                        kw.insert(k.str(), v);
                    }
                }
            }
        }
        CtorArgs { pos, kw }
    }

    pub(crate) fn get(&self, idx: usize, name: &str) -> Option<PyObjectRef> {
        self.pos
            .get(idx)
            .cloned()
            .or_else(|| self.kw.get(name).cloned())
    }

    pub(crate) fn get_i64(&self, idx: usize, name: &str, default: i64) -> i64 {
        self.get(idx, name)
            .and_then(|v| v.as_i64())
            .unwrap_or(default)
    }
}

thread_local! {
    static TZINFO_TYPE: std::cell::RefCell<Option<PyObjectRef>> = std::cell::RefCell::new(None);
}

pub(crate) fn get_tzinfo_type() -> PyObjectRef {
    let existing = TZINFO_TYPE.with(|c| c.borrow().clone());
    if let Some(t) = existing {
        return t;
    }
    let mut type_dict: HashMap<String, PyObjectRef> = HashMap::new();
    macro_rules! bf {
        ($name:expr, $f:expr) => {
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: $name.to_string(),
                func: $f,
            })
        };
    }
    type_dict.insert_str(
        "utcoffset",
        bf!("utcoffset", |_args| Err(PyError::runtime_error(
            "tzinfo subclasses must override utcoffset()"
        ))),
    );
    type_dict.insert_str(
        "dst",
        bf!("dst", |_args| Err(PyError::runtime_error(
            "tzinfo subclasses must override dst()"
        ))),
    );
    type_dict.insert_str(
        "tzname",
        bf!("tzname", |_args| Err(PyError::runtime_error(
            "tzinfo subclasses must override tzname()"
        ))),
    );
    let typ = PyObjectRef::new(PyObject::Type {
        name: "tzinfo".to_string(),
        dict: Box::new(str_map_to_typedict(type_dict)),
        bases: vec![],
        mro: vec![],
    });
    TZINFO_TYPE.with(|c| {
        *c.borrow_mut() = Some(typ.clone());
    });
    typ
}
