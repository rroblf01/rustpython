// Split from src/object/memoryview.rs — format/itemsize and source-bytes helpers.
use super::*;
use crate::object::*;

pub(crate) fn mv_itemsize(format: &str) -> usize {
    match format {
        "b" | "B" | "c" | "?" | "x" => 1,
        "h" | "H" | "e" => 2,
        "i" | "I" | "l" | "L" | "f" => 4,
        "q" | "Q" | "d" | "n" | "N" => 8,
        _ => 1,
    }
}

pub(crate) fn mv_total_items(shape: &[usize]) -> usize {
    if shape.is_empty() {
        1
    } else {
        shape.iter().product()
    }
}

pub(crate) fn array_elem_to_bytes(typecode: char, val: f64) -> Vec<u8> {
    let isz = mv_itemsize(&typecode.to_string());
    if array_typecode_is_float(typecode) {
        if isz == 4 {
            (val as f32).to_ne_bytes().to_vec()
        } else {
            val.to_ne_bytes().to_vec()
        }
    } else {
        let n = val as i64;
        match isz {
            1 => vec![n as u8],
            2 => (n as i16).to_ne_bytes().to_vec(),
            4 => (n as i32).to_ne_bytes().to_vec(),
            _ => n.to_ne_bytes().to_vec(),
        }
    }
}

pub(crate) fn array_bytes_to_elem(typecode: char, bytes: &[u8]) -> f64 {
    if array_typecode_is_float(typecode) {
        if bytes.len() == 4 {
            f32::from_ne_bytes(bytes.try_into().unwrap()) as f64
        } else {
            f64::from_ne_bytes(bytes.try_into().unwrap())
        }
    } else {
        match bytes.len() {
            1 => bytes[0] as i64 as f64,
            2 => i16::from_ne_bytes(bytes.try_into().unwrap()) as f64,
            4 => i32::from_ne_bytes(bytes.try_into().unwrap()) as f64,
            _ => i64::from_ne_bytes(bytes.try_into().unwrap()) as f64,
        }
    }
}

pub(crate) fn mv_source_bytes(source: &PyObjectRef) -> Vec<u8> {
    if let Some(backing) = crate::object::native_backing_of(source) {
        match &*backing.borrow() {
            PyObject::Bytes(b) => return b.clone(),
            PyObject::ByteArray(b) => return b.clone(),
            PyObject::Array(arr) => {
                let mut out =
                    Vec::with_capacity(arr.data.len() * mv_itemsize(&arr.typecode.to_string()));
                for &v in &arr.data {
                    out.extend(array_elem_to_bytes(arr.typecode, v));
                }
                return out;
            }
            _ => {}
        }
    }
    match &*source.borrow() {
        PyObject::Bytes(b) => b.clone(),
        PyObject::ByteArray(b) => b.clone(),
        PyObject::Array(arr) => {
            let mut out =
                Vec::with_capacity(arr.data.len() * mv_itemsize(&arr.typecode.to_string()));
            for &v in &arr.data {
                out.extend(array_elem_to_bytes(arr.typecode, v));
            }
            out
        }
        _ => Vec::new(),
    }
}

pub(crate) fn mv_write_bytes(source: &PyObjectRef, offset: usize, data: &[u8]) -> PyResult<()> {
    if let Some(backing) = crate::object::native_backing_of(source) {
        return mv_write_bytes(&backing, offset, data);
    }
    match &mut *source.borrow_mut() {
        PyObject::ByteArray(b) => {
            if offset + data.len() > b.len() {
                return Err(PyError::index_error("memoryview assignment out of range"));
            }
            b[offset..offset + data.len()].copy_from_slice(data);
            Ok(())
        }
        PyObject::Array(arr) => {
            let isz = mv_itemsize(&arr.typecode.to_string());
            if data.len() % isz != 0 || offset % isz != 0 {
                return Err(PyError::value_error(
                    "memoryview assignment: lvalue and rvalue have different structures",
                ));
            }
            let start_elem = offset / isz;
            let n_elems = data.len() / isz;
            if start_elem + n_elems > arr.data.len() {
                return Err(PyError::index_error("memoryview assignment out of range"));
            }
            for i in 0..n_elems {
                arr.data[start_elem + i] =
                    array_bytes_to_elem(arr.typecode, &data[i * isz..(i + 1) * isz]);
            }
            Ok(())
        }
        _ => Err(PyError::type_error("cannot modify read-only memory")),
    }
}

pub(crate) fn nest_list(items: &[PyObjectRef], shape: &[usize]) -> PyObjectRef {
    if shape.len() <= 1 {
        return py_list(items.to_vec());
    }
    let inner_shape = &shape[1..];
    let inner_size = mv_total_items(inner_shape);
    let mut rows = Vec::with_capacity(shape[0]);
    for i in 0..shape[0] {
        rows.push(nest_list(
            &items[i * inner_size..(i + 1) * inner_size],
            inner_shape,
        ));
    }
    py_list(rows)
}
