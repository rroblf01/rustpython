// Split from src/object/core.rs — SmallStr / RefOrOwned / BuiltinFunc / counters.
use crate::object::{PyObject, PyObjectRef};
use std::sync::atomic::AtomicUsize;

pub type BuiltinFunc = fn(&[PyObjectRef]) -> crate::object::PyResult<PyObjectRef>;

pub static ALLOC_COUNT: AtomicUsize = AtomicUsize::new(0);
pub static IMM_COUNT: AtomicUsize = AtomicUsize::new(0);

pub enum RefOrOwned<'a> {
    Ref(std::cell::Ref<'a, PyObject>),
    Owned(PyObject),
    Borrow(&'a PyObject),
}

impl<'a> std::ops::Deref for RefOrOwned<'a> {
    type Target = PyObject;
    fn deref(&self) -> &PyObject {
        match self {
            RefOrOwned::Ref(r) => &**r,
            RefOrOwned::Owned(o) => o,
            RefOrOwned::Borrow(b) => b,
        }
    }
}

#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct SmallStr {
    data: [u8; 15],
    len: u8,
}

impl SmallStr {
    pub fn new(s: &str) -> Option<Self> {
        let bytes = s.as_bytes();
        if bytes.len() > 15 { return None; }
        let mut data = [0u8; 15];
        data[..bytes.len()].copy_from_slice(bytes);
        Some(SmallStr { data, len: bytes.len() as u8 })
    }
    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.data[..self.len as usize]).expect("SmallStr: invalid UTF-8 data")
    }
}
