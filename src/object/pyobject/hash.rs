// Extracted from pyobject.rs — PyObject::hash.
use super::*;

impl PyObject {
    pub fn hash(&self) -> PyResult<usize> {
        match self {
            PyObject::None => Ok(0),
            PyObject::Bool(b) => Ok(if *b { 1 } else { 0 }),
            PyObject::Int(i) => Ok(hash_bigint(i)),
            // A whole-number float must hash IDENTICALLY to the equal int
            // (`1.0 == 1` is true, per the numeric-tower equality fix above,
            // and Python's dict/set invariant requires `a == b => hash(a) ==
            // hash(b)` — otherwise `{1: 'x'}[1.0]` raises `KeyError` even
            // though `1.0 in {1: 'x'}` reports the key as present via `==`).
            // Reuses Int's own (already-established) hash function directly
            // rather than reimplementing CPython's real mod-2**61-1 float
            // hash algorithm — this covers the overwhelmingly common case
            // (whole-number float dict/set keys) without changing Int's own
            // existing hash values. Non-whole-number floats keep the prior
            // bit-pattern hash (internally consistent, just not
            // cross-type-matching — which only matters for fractional
            // int/float equality, impossible for finite non-whole floats).
            PyObject::Float(f) => {
                // NaN hashes to 0 (see the SmallFloat arm in
                // `PyObjectRef::hash` — this enum method has no handle to
                // compute an object-identity hash). Finite values use
                // CPython's `_Py_HashDouble` so whole-number floats hash
                // identically to the equal int AND `hash(inf) == 314159`.
                if f.is_nan() {
                    Ok(0)
                } else {
                    Ok(hash_double(*f))
                }
            }
            PyObject::Complex(re, im) => {
                let real_hash = PyObject::Float(*re).hash()?;
                if *im == 0.0 {
                    Ok(real_hash)
                } else {
                    let imag_hash = PyObject::Float(*im).hash()?;
                    let combined =
                        (real_hash as i64).wrapping_add(1000003i64.wrapping_mul(imag_hash as i64));
                    Ok((if combined == -1 { -2 } else { combined }) as usize)
                }
            }
            PyObject::Str(s) => Ok(py_hash_str(s)),
            PyObject::Bytes(b) => Ok(py_hash_bytes(b)),
            PyObject::Range { start, stop, step } => {
                // CPython hashes (length, start, step) — NOT stop, so equal
                // ranges hash equal regardless of differing stops.
                let length = crate::object::ops_contains::range_len_values(start, stop, step);
                let one = num_bigint::BigInt::from(1);
                let mut h: usize = 0x345678;
                let mix = |h: usize, v: &num_bigint::BigInt| -> usize {
                    h.wrapping_mul(1000003)
                        .wrapping_add(v.to_usize().unwrap_or(0))
                };
                h = mix(h, &length);
                let zero = num_bigint::BigInt::from(0);
                if length != zero {
                    h = mix(h, start);
                    if length != one {
                        h = mix(h, step);
                    }
                }
                Ok(h)
            }
            PyObject::Tuple(items) => {
                // CPython 3.14's exact tuple hash (xxHash-style, so
                // hash((...)) matches real CPython on 64-bit platforms).
                const PRIME_1: u64 = 11400714785074694791;
                const PRIME_2: u64 = 14029467366897019727;
                const PRIME_5: u64 = 2870177450012600261;
                let mut acc: u64 = PRIME_5;
                for item in items {
                    let lane = item.hash()? as u64;
                    acc = acc.wrapping_add(lane.wrapping_mul(PRIME_2));
                    acc = acc.rotate_left(31);
                    acc = acc.wrapping_mul(PRIME_1);
                }
                let len = items.len() as u64;
                acc = acc.wrapping_add(len ^ (PRIME_5 ^ 3527539));
                if acc == u64::MAX {
                    acc = 1546275796;
                }
                Ok(acc as usize)
            }
            PyObject::FrozenSet(items) => {
                let mut h: usize = 0x987654;
                for item in items.to_vec() {
                    h = h.wrapping_mul(1000003).wrapping_add(item.hash()?);
                }
                Ok(h)
            }
            PyObject::Instance { typ, dict } => {
                // Check for __hash__ method (walking the MRO)
                let f = lookup_dunder_via_mro(typ, "__hash__");
                if let Some(f) = f {
                    let result = call_bound_method(
                        f,
                        PyObjectRef::new(PyObject::Instance {
                            typ: typ.clone(),
                            dict: dict.clone(),
                        }),
                        vec![],
                    )?;
                    let n = result.borrow();
                    if let PyObject::Int(i) = &*n {
                        let bytes = i.to_signed_bytes_le();
                        let mut h: usize = 0;
                        for (j, &b) in bytes.iter().enumerate() {
                            h ^= (b as usize) << ((j % (std::mem::size_of::<usize>())) * 8);
                        }
                        Ok(h)
                    } else {
                        Err(PyError::type_error("__hash__ should return an integer"))
                    }
                } else if let Some(native) = dict.get(NATIVE_BACKING_KEY) {
                    native.hash()
                } else {
                    Err(PyError::type_error(format!(
                        "unhashable type: '{}'",
                        self.type_name()
                    )))
                }
            }
            PyObject::Array(arr) => {
                let mut h: usize = 0xabcdef;
                for &v in &arr.data {
                    let bits = v.to_bits();
                    h = h.wrapping_mul(1000003).wrapping_add(bits as usize);
                }
                Ok(h)
            }
            PyObject::Slice { start, stop, step } => {
                let mut h: usize = 0x345679;
                h = h.wrapping_mul(1000003).wrapping_add(start.hash()?);
                h = h.wrapping_mul(1000003).wrapping_add(stop.hash()?);
                h = h.wrapping_mul(1000003).wrapping_add(step.hash()?);
                Ok(h)
            }
            PyObject::CompiledRegex { pattern, flags, .. } => {
                let mut h: usize = 0x123456;
                for b in pattern.bytes() {
                    h = h.wrapping_mul(1000003).wrapping_add(b as usize);
                }
                h = h.wrapping_mul(1000003).wrapping_add(*flags as usize);
                Ok(h)
            }
            // Functions, types, modules, etc. are hashable by identity in
            // real Python (there's no reasonable structural hash for them,
            // but there's no reason they should be unhashable either — code
            // that registers callbacks in a set/dict, e.g. Django's check
            // registry, relies on this). `self` here is `&PyObject` reached
            // via a `Ref` guard borrowed from the object's own Rc, so its
            // address is stable across calls as long as callers don't
            // reconstruct a throwaway clone first (unlike the Instance case
            // above, which needed its own fix for exactly that reason).
            // Iterator objects (and anything else with no sensible
            // structural equality of its own) are hashable BY IDENTITY in
            // real Python — hashability is opt-OUT (only mutable
            // containers like `list`/`dict`/`set` explicitly disable it),
            // not opt-in. These previously fell to the generic `_`
            // catch-all below, making every one of them unhashable —
            // found via CPython's own `test_hash.py::test_hashes`, whose
            // `hashes_to_check` list includes `enumerate(...)`, `iter(an_
            // object_with_only___getitem__)` (this interpreter's own
            // `GetItemIter`), and `iter(callable, sentinel)` (`
            // CallSentinelIter`).
            PyObject::WeakRef { target, hash_cache, .. } => {
                if let Some(rc) = target.upgrade() {
                    let h = PyObjectRef::Imm(rc.clone()).hash()?;
                    if hash_cache.borrow().is_none() {
                        *hash_cache.borrow_mut() = Some(h);
                    }
                    return Ok(h);
                } else {
                    if let Some(h) = *hash_cache.borrow() {
                        return Ok(h);
                    }
                    return Err(PyError::type_error("weak object has gone away"));
                }
            }
            PyObject::WeakProxy { .. } => {
                return Err(PyError::type_error("unhashable type: 'weakproxy'"));
            }
            PyObject::Function(_)
            | PyObject::BuiltinMethod { .. }
            | PyObject::Type { .. }
            | PyObject::Module { .. }
            | PyObject::BoundMethod { .. }
            | PyObject::EnumerateIter { .. }
            | PyObject::GetItemIter { .. }
            | PyObject::CallSentinelIter { .. }
            | PyObject::ListIter { .. }
            | PyObject::RangeIter { .. }
            | PyObject::DequeIter { .. }
            | PyObject::DequeRevIter { .. }
            | PyObject::DictIter { .. }
            | PyObject::DictValuesIter { .. }
            | PyObject::DictItemsIter { .. }
            | PyObject::DictRevIter { .. }
            | PyObject::MapIterator { .. }
            | PyObject::FilterIterator { .. }
            | PyObject::ZipIterator { .. }
            | PyObject::CycleIter { .. }
            | PyObject::GroupByIter { .. }
            | PyObject::Socket { .. } => Ok(self as *const PyObject as usize),
            // A READ-ONLY `memoryview` (over `bytes`) IS hashable in real
            // Python, hashing exactly like the equivalent `bytes` content
            // (`hash(memoryview(b'x')) == hash(b'x')`) — a WRITABLE one
            // (over `bytearray`) is NOT, matching `bytearray`'s own
            // unhashability. Previously fell to the generic `_` catch-all,
            // making EVERY memoryview unhashable regardless of
            // readonly-ness. Found via CPython's own `test_hash.py`.
            PyObject::MemoryView { readonly, .. } => {
                if !readonly {
                    // Real CPython raises `ValueError` here, NOT `TypeError`
                    // — a writable memoryview isn't "unhashable" in the
                    // usual sense (real CPython's own message: "cannot hash
                    // writable memoryview object"), it's specifically
                    // disallowed because a live view over mutable memory
                    // would violate hash-stability if the buffer changed.
                    return Err(PyError::value_error(
                        "cannot hash writable memoryview object",
                    ));
                }
                let self_ref = PyObjectRef::new(self.clone());
                let bytes = mv_tobytes(&self_ref)?;
                PyObject::Bytes(bytes).hash()
            }
            PyObject::BuiltinFunction { name, func } => {
                let h1 = py_hash_str(name);
                let h2 = *func as *const () as usize;
                Ok(h1.wrapping_mul(1000003).wrapping_add(h2))
            }
            PyObject::Closure(_) => Err(PyError::type_error(format!(
                "unhashable type: '{}'",
                self.type_name()
            ))),
            _ => Err(PyError::type_error(format!(
                "unhashable type: '{}'",
                self.type_name()
            ))),
        }
    }
}
