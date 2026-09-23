//! `re.RegexFlag` — IntFlag-like flag values (see CPython `re` + `enum`).
//!
//! Members (definition order, used by repr): ASCII=256, IGNORECASE=2,
//! LOCALE=4, UNICODE=32, MULTILINE=8, DOTALL=16, VERBOSE=64, DEBUG=128,
//! plus NOFLAG=0 (name for the zero value). No TEMPLATE (removed in
//! CPython 3.11+). Stored as `PyObject::RegexFlag(i64)`, always `>= 0`
//! (the KEEP boundary wraps `~` and negative construction into the
//! positive range, exactly like CPython: `int(~re.I) == 509`).
use super::{PyObject, PyObjectRef, PyResult};

/// (value, name) in CPython definition order — repr lists members in
/// this order, NOT sorted by value.
const MEMBERS: &[(i64, &str)] = &[
    (256, "ASCII"),
    (2, "IGNORECASE"),
    (4, "LOCALE"),
    (32, "UNICODE"),
    (8, "MULTILINE"),
    (16, "DOTALL"),
    (64, "VERBOSE"),
    (128, "DEBUG"),
];

const FLAG_MASK: i64 = 0x1FE;

/// KEEP-boundary wrap: non-negative values pass through; negatives wrap
/// into the positive range via `max(all_bits+1, 2**bit_length) + value`
/// (mirrors `IntFlag._missing_` with `_boundary_ is KEEP`).
pub fn regexflag_wrap(v: i64) -> i64 {
    if v >= 0 {
        return v;
    }
    // Python `(-n).bit_length() == (n-1).bit_length()`; here `~v == -v-1`.
    let mag = (!v) as u64;
    let bl = 64 - mag.leading_zeros();
    let pow = if bl >= 64 { u64::MAX } else { 1u64 << bl };
    let base = std::cmp::max(512u64, pow);
    (base as i64).wrapping_add(v)
}

/// IntFlag repr with `re.`-qualified names (mirrors CPython
/// `enum.global_flag_repr` for the `re` module):
/// - `0` → `re.NOFLAG`
/// - single member → `re.NAME`
/// - member combination → `re.A|re.B`
/// - with unknown bits → `...|0xNN` (hex, lowercase)
/// - pure unknown → `re.RegexFlag(N)` (decimal)
pub fn regexflag_repr(v: i64) -> String {
    let v = regexflag_wrap(v);
    if v == 0 {
        return "re.NOFLAG".to_string();
    }
    let mut parts: Vec<String> = Vec::new();
    let mut combined: i64 = 0;
    for (val, name) in MEMBERS {
        if (*val & v) == *val {
            parts.push(format!("re.{}", name));
            combined |= *val;
        }
    }
    if parts.is_empty() {
        return format!("re.RegexFlag({})", v);
    }
    let unknown = v ^ combined;
    if unknown != 0 {
        parts.push(format!("0x{:x}", unknown));
    }
    parts.join("|")
}

/// Build a (wrapped) RegexFlag value.
pub fn regexflag_new(v: i64) -> PyObjectRef {
    PyObjectRef::imm(PyObject::RegexFlag(regexflag_wrap(v)))
}

/// `~v` with KEEP wrap (CPython: `int(~re.I) == 509`).
pub fn regexflag_invert(v: i64) -> PyObjectRef {
    regexflag_new(!v)
}

/// `|`/`&`/`^` when either side is a RegexFlag and the other is
/// int-like (Flag/int/bool). Returns `None` to fall through to the
/// normal dispatch (e.g. Flag with float/str → TypeError). Results are
/// always `>= 0` (inputs are), so no wrap is needed.
pub fn regexflag_binop(
    a: &PyObjectRef,
    b: &PyObjectRef,
    f: impl Fn(i64, i64) -> i64,
) -> Option<PyResult<PyObjectRef>> {
    let a_is = matches!(&*a.borrow(), PyObject::RegexFlag(_));
    let b_is = matches!(&*b.borrow(), PyObject::RegexFlag(_));
    if !a_is && !b_is {
        return None;
    }
    let (av, bv) = (a.as_i64()?, b.as_i64()?);
    Some(Ok(PyObjectRef::imm(PyObject::RegexFlag(f(av, bv)))))
}
