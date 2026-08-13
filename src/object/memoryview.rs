// A real `memoryview`, replacing a former alias to a cloned `bytearray`
// (no `.cast()`, no `.format`/`.shape`/`.itemsize`, no multi-dimensional
// support, and mutations through it never reflected back into the
// original buffer). See `PyObject::MemoryView`'s own doc comment
// (`pyobject.rs`) for the field layout and write-through sharing rationale.
use super::*;

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

fn array_elem_to_bytes(typecode: char, val: f64) -> Vec<u8> {
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

fn array_bytes_to_elem(typecode: char, bytes: &[u8]) -> f64 {
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

/// A read-only snapshot of the source's raw bytes — fine for reads (the
/// clone is thrown away immediately), NOT used for writes (see
/// `mv_write_bytes`, which mutates the source in place instead).
pub(crate) fn mv_source_bytes(source: &PyObjectRef) -> Vec<u8> {
    match &*source.borrow() {
        PyObject::Bytes(b) => b.clone(),
        PyObject::ByteArray(b) => b.clone(),
        // `array.array` implements the buffer protocol too (real Python:
        // `memoryview(array.array('i', [1,2,3]))` works directly) — was
        // entirely unsupported, raising `TypeError: memoryview: a
        // bytes-like object is required, not 'array'` for every one of
        // `test_memoryview.py`'s own `BaseArrayMemoryTests`-derived cases.
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

pub(crate) fn mv_fields(
    v: &PyObjectRef,
) -> PyResult<(PyObjectRef, String, Vec<usize>, usize, usize, bool)> {
    if let PyObject::MemoryView {
        source,
        format,
        shape,
        itemsize,
        offset,
        readonly,
    } = &*v.borrow()
    {
        Ok((
            source.clone(),
            format.clone(),
            shape.clone(),
            *itemsize,
            *offset,
            *readonly,
        ))
    } else {
        Err(PyError::type_error("not a memoryview"))
    }
}

fn mv_decode_elem(format: &str, bytes: &[u8]) -> PyObjectRef {
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
        // 'B' and anything unrecognized: plain unsigned byte.
        _ => py_int(bytes[0] as i64),
    }
}

fn mv_encode_elem(format: &str, val: &PyObjectRef) -> PyResult<Vec<u8>> {
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
                _ => (n as i32).to_ne_bytes().to_vec(), // i/I/l/L
            }
        }
    })
}

fn nest_list(items: &[PyObjectRef], shape: &[usize]) -> PyObjectRef {
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

pub fn builtin_memoryview(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() != 1 {
        return Err(PyError::type_error(
            "memoryview() takes exactly one argument",
        ));
    }
    let existing = if let PyObject::MemoryView {
        source,
        format,
        shape,
        itemsize,
        offset,
        readonly,
    } = &*args[0].borrow()
    {
        Some((
            source.clone(),
            format.clone(),
            shape.clone(),
            *itemsize,
            *offset,
            *readonly,
        ))
    } else {
        None
    };
    if let Some((source, format, shape, itemsize, offset, readonly)) = existing {
        return Ok(PyObjectRef::new(PyObject::MemoryView {
            source,
            format,
            shape,
            itemsize,
            offset,
            readonly,
        }));
    }
    let (readonly, format, len) = match &*args[0].borrow() {
        PyObject::Bytes(b) => (true, "B".to_string(), b.len()),
        PyObject::ByteArray(b) => (false, "B".to_string(), b.len()),
        // `array.array` uses its OWN typecode as the memoryview's format
        // (matching real Python: `memoryview(array.array('i', ...)).format
        // == 'i'`), not the generic byte view `bytes`/`bytearray` get.
        PyObject::Array(arr) => (false, arr.typecode.to_string(), arr.data.len()),
        other => {
            return Err(PyError::type_error(format!(
                "memoryview: a bytes-like object is required, not '{}'",
                other.type_name()
            )))
        }
    };
    let itemsize = mv_itemsize(&format);
    Ok(PyObjectRef::new(PyObject::MemoryView {
        source: args[0].clone(),
        format,
        shape: vec![len],
        itemsize,
        offset: 0,
        readonly,
    }))
}

pub(crate) fn mv_len(v: &PyObjectRef) -> PyResult<usize> {
    let (_, _, shape, ..) = mv_fields(v)?;
    Ok(shape.first().copied().unwrap_or(0))
}

pub(crate) fn mv_nbytes(v: &PyObjectRef) -> PyResult<usize> {
    let (_, _, shape, itemsize, ..) = mv_fields(v)?;
    Ok(itemsize * mv_total_items(&shape))
}

pub(crate) fn mv_tobytes(v: &PyObjectRef) -> PyResult<Vec<u8>> {
    let (source, _, shape, itemsize, offset, _) = mv_fields(v)?;
    let total = itemsize * mv_total_items(&shape);
    let all = mv_source_bytes(&source);
    if offset + total > all.len() {
        return Err(PyError::index_error("memoryview out of range"));
    }
    Ok(all[offset..offset + total].to_vec())
}

fn mv_tolist_impl(v: &PyObjectRef) -> PyResult<PyObjectRef> {
    let (source, format, shape, itemsize, offset, _) = mv_fields(v)?;
    let all = mv_source_bytes(&source);
    let n = mv_total_items(&shape);
    if offset + n * itemsize > all.len() {
        return Err(PyError::index_error("memoryview out of range"));
    }
    let mut items = Vec::with_capacity(n);
    for i in 0..n {
        let start = offset + i * itemsize;
        items.push(mv_decode_elem(&format, &all[start..start + itemsize]));
    }
    Ok(nest_list(&items, &shape))
}

fn mv_cast_impl(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 2 {
        return Err(PyError::type_error("cast() takes at least 1 argument"));
    }
    let (source, _cur_format, cur_shape, cur_itemsize, offset, readonly) = mv_fields(&args[0])?;
    let new_format = match &*args[1].borrow() {
        PyObject::Str(s) => s.to_string(),
        _ => return Err(PyError::type_error("format argument must be a string")),
    };
    let total_bytes = cur_itemsize * mv_total_items(&cur_shape);
    let new_itemsize = mv_itemsize(&new_format);
    if new_itemsize == 0 {
        return Err(PyError::value_error(format!("memoryview: destination format must be a native single character format prefixed with an optional '@'")));
    }
    let new_shape: Vec<usize> = if args.len() > 2 && !matches!(&*args[2].borrow(), PyObject::None) {
        match &*args[2].borrow() {
            PyObject::Tuple(items) | PyObject::List(items) => items
                .iter()
                .map(|v| v.as_i64().unwrap_or(0) as usize)
                .collect(),
            _ => return Err(PyError::type_error("shape must be a list or tuple")),
        }
    } else {
        if total_bytes % new_itemsize != 0 {
            return Err(PyError::type_error(
                "memoryview: length is not a multiple of itemsize",
            ));
        }
        vec![total_bytes / new_itemsize]
    };
    let expected_bytes = new_itemsize * mv_total_items(&new_shape);
    if expected_bytes != total_bytes {
        return Err(PyError::type_error(
            "memoryview: length is not a multiple of itemsize",
        ));
    }
    Ok(PyObjectRef::new(PyObject::MemoryView {
        source,
        format: new_format,
        shape: new_shape,
        itemsize: new_itemsize,
        offset,
        readonly,
    }))
}

pub(crate) fn mv_getattr(name: &str) -> Option<PyObjectRef> {
    macro_rules! method {
        ($f:expr) => {
            Some(PyObjectRef::imm(PyObject::BuiltinMethod {
                name: name.to_string(),
                func: $f,
                self_obj: PyObjectRef::new(PyObject::None),
            }))
        };
    }
    match name {
        "cast" => method!(|args| mv_cast_impl(args)),
        "tobytes" | "tostring" => {
            method!(|args| Ok(PyObjectRef::imm(PyObject::Bytes(mv_tobytes(&args[0])?))))
        }
        "tolist" => method!(|args| mv_tolist_impl(&args[0])),
        "hex" => method!(|args| {
            let bytes = mv_tobytes(&args[0])?;
            Ok(py_str(
                &bytes
                    .iter()
                    .map(|b| format!("{:02x}", b))
                    .collect::<String>(),
            ))
        }),
        "release" => method!(|_args| Ok(py_none())),
        "__enter__" => method!(|args| Ok(args[0].clone())),
        "__exit__" => method!(|_args| Ok(py_bool(false))),
        "__len__" => method!(|args| Ok(py_int(mv_len(&args[0])? as i64))),
        _ => None,
    }
}

pub(crate) fn mv_getprop(v: &PyObjectRef, name: &str) -> Option<PyResult<PyObjectRef>> {
    let (source, format, shape, itemsize, offset, readonly) = match mv_fields(v) {
        Ok(f) => f,
        Err(e) => return Some(Err(e)),
    };
    match name {
        "format" => Some(Ok(py_str(&format))),
        "itemsize" => Some(Ok(py_int(itemsize as i64))),
        "shape" => Some(Ok(py_tuple(
            shape.iter().map(|&n| py_int(n as i64)).collect(),
        ))),
        "ndim" => Some(Ok(py_int(shape.len() as i64))),
        "nbytes" => Some(Ok(py_int((itemsize * mv_total_items(&shape)) as i64))),
        "readonly" => Some(Ok(py_bool(readonly))),
        "contiguous" | "c_contiguous" => Some(Ok(py_bool(true))),
        "f_contiguous" => Some(Ok(py_bool(shape.len() <= 1))),
        "obj" => Some(Ok(source)),
        "strides" => {
            // Simple C-contiguous row-major strides.
            let mut strides = Vec::with_capacity(shape.len());
            let mut acc = itemsize;
            for &dim in shape.iter().rev() {
                strides.push(acc as i64);
                acc *= dim.max(1);
            }
            strides.reverse();
            Some(Ok(py_tuple(strides.into_iter().map(py_int).collect())))
        }
        _ => {
            let _ = offset;
            None
        }
    }
}

pub(crate) fn mv_getitem(v: &PyObjectRef, index: &PyObjectRef) -> PyResult<PyObjectRef> {
    let (source, format, shape, itemsize, offset, _readonly) = mv_fields(v)?;
    let all = mv_source_bytes(&source);
    if let PyObject::Slice { start, stop, step } = &*index.borrow() {
        let len = shape.first().copied().unwrap_or(0);
        let (start_val, stop_val, step_val) = extract_slice_fields(start, stop, step)?;
        if step_val != 1 {
            return Err(PyError::type_error(
                "memoryview slicing with step != 1 is not supported",
            ));
        }
        let (start_n, stop_n) = normalize_slice_bounds(start_val, stop_val, step_val, len);
        let count = (stop_n - start_n).max(0) as usize;
        let mut new_shape = shape.clone();
        new_shape[0] = count;
        let row_size: usize = itemsize * mv_total_items(&shape[1..]);
        return Ok(PyObjectRef::new(PyObject::MemoryView {
            source: source.clone(),
            format,
            shape: new_shape,
            itemsize,
            offset: offset + (start_n as usize) * row_size,
            readonly: _readonly,
        }));
    }
    let i = index
        .as_i64()
        .ok_or_else(|| PyError::type_error("memoryview: invalid slice key"))?;
    let len = shape.first().copied().unwrap_or(0) as i64;
    let i = if i < 0 { len + i } else { i };
    if i < 0 || i >= len {
        return Err(PyError::index_error("index out of bounds"));
    }
    if shape.len() <= 1 {
        let start = offset + (i as usize) * itemsize;
        if start + itemsize > all.len() {
            return Err(PyError::index_error("memoryview index out of range"));
        }
        Ok(mv_decode_elem(&format, &all[start..start + itemsize]))
    } else {
        let row_size = mv_total_items(&shape[1..]);
        Ok(PyObjectRef::new(PyObject::MemoryView {
            source: source.clone(),
            format,
            shape: shape[1..].to_vec(),
            itemsize,
            offset: offset + (i as usize) * row_size * itemsize,
            readonly: _readonly,
        }))
    }
}

pub(crate) fn mv_setitem(v: &PyObjectRef, index: &PyObjectRef, value: PyObjectRef) -> PyResult<()> {
    let (source, format, shape, itemsize, offset, readonly) = mv_fields(v)?;
    if readonly {
        return Err(PyError::type_error("cannot modify read-only memory"));
    }
    if shape.len() > 1 {
        return Err(PyError::type_error(
            "memoryview assignment to multi-dimensional views is not supported",
        ));
    }
    let i = index
        .as_i64()
        .ok_or_else(|| PyError::type_error("memoryview: invalid slice key"))?;
    let len = shape.first().copied().unwrap_or(0) as i64;
    let i = if i < 0 { len + i } else { i };
    if i < 0 || i >= len {
        return Err(PyError::index_error("index out of bounds"));
    }
    let bytes = mv_encode_elem(&format, &value)?;
    mv_write_bytes(&source, offset + (i as usize) * itemsize, &bytes)
}

pub(crate) fn mv_equals(a: &PyObjectRef, b: &PyObjectRef) -> bool {
    let a_bytes = match mv_tobytes(a) {
        Ok(b) => b,
        Err(_) => return false,
    };
    match &*b.borrow() {
        PyObject::Bytes(bb) => a_bytes == *bb,
        PyObject::ByteArray(bb) => a_bytes == *bb,
        PyObject::MemoryView { .. } => mv_tobytes(b).map(|bb| bb == a_bytes).unwrap_or(false),
        _ => false,
    }
}
