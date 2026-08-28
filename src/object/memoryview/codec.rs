// Split from src/object/memoryview.rs — decode/encode and buffer-flags helpers.
use super::*;
use crate::object::*;
use num_traits::ToPrimitive;

pub(crate) fn mv_decode_elem(format: &str, bytes: &[u8]) -> PyObjectRef {
    match format {
        "b" => py_int(bytes[0] as i8 as i64),
        "c" => PyObjectRef::imm(PyObject::Bytes(vec![bytes[0]])),
        "?" => py_bool(bytes[0] != 0),
        "h" => py_int(i16::from_ne_bytes([bytes[0], bytes[1]]) as i64),
        "H" => py_int(u16::from_ne_bytes([bytes[0], bytes[1]]) as i64),
        "i" | "l" => py_int(i32::from_ne_bytes(bytes[..4].try_into().unwrap()) as i64),
        "I" | "L" => py_int(u32::from_ne_bytes(bytes[..4].try_into().unwrap()) as i64),
        "q" | "n" => py_int(i64::from_ne_bytes(bytes[..8].try_into().unwrap())),
        "Q" | "N" => py_int(u64::from_ne_bytes(bytes[..8].try_into().unwrap())),
        "f" => py_float(f32::from_ne_bytes(bytes[..4].try_into().unwrap()) as f64),
        "d" => py_float(f64::from_ne_bytes(bytes[..8].try_into().unwrap())),
        _ => py_int(bytes[0] as i64),
    }
}

pub(crate) fn mv_encode_elem(format: &str, val: &PyObjectRef) -> PyResult<Vec<u8>> {
    let bad = || PyError::type_error("memoryview: invalid type for format");
    Ok(match format {
        "c" => match &*val.borrow() {
            PyObject::Bytes(b) if b.len() == 1 => vec![b[0]],
            _ => {
                return Err(PyError::type_error(
                    "memoryview: invalid value for format 'c'",
                ))
            }
        },
        "?" => vec![if val.truthy() { 1 } else { 0 }],
        "f" => (val.as_f64().ok_or_else(bad)? as f32)
            .to_ne_bytes()
            .to_vec(),
        "d" => val.as_f64().ok_or_else(bad)?.to_ne_bytes().to_vec(),
        _ => {
            let n = val.as_i64().ok_or_else(bad)?;
            match format {
                "b" | "B" => vec![n as u8],
                "h" | "H" => (n as i16).to_ne_bytes().to_vec(),
                "q" | "Q" | "n" | "N" => n.to_ne_bytes().to_vec(),
                _ => (n as i32).to_ne_bytes().to_vec(),
            }
        }
    })
}

pub(crate) fn is_picklebuffer_obj(obj: &PyObjectRef) -> Option<(PyObjectRef, bool)> {
    if let PyObject::Instance { typ, dict } = &*obj.borrow() {
        let is_pb = if let PyObject::Type { name, .. } = &*typ.borrow() {
            name == "PickleBuffer"
        } else {
            false
        };
        if is_pb {
            let released = dict
                .get("_released")
                .map(|v| v.truthy())
                .unwrap_or(false);
            let underlying = dict.get("_obj").cloned().unwrap_or_else(py_none);
            return Some((underlying, released));
        }
    }
    None
}

pub(crate) fn extract_flags_for_buffer(obj: &PyObjectRef) -> PyResult<i64> {
    let flags_big = crate::object::to_index(obj)?;
    if flags_big.to_i64().is_none() {
        return Err(PyError::overflow_error("Python int too large to convert to C int"));
    }
    let v = flags_big.to_i64().unwrap();
    if v > i32::MAX as i64 || v < i32::MIN as i64 {
        return Err(PyError::overflow_error("Python int too large to convert to C int"));
    }
    Ok(v)
}

pub(crate) fn check_buffer_flags(flags: i64) -> PyResult<()> {
    // READ=0x100, WRITE=0x200 are invalid for getbuffer (CPython raises SystemError)
    if flags & 0x100 != 0 || flags & 0x200 != 0 {
        return Err(PyError::system_error("invalid buffer flags"));
    }
    Ok(())
}
