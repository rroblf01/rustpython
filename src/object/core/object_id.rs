// Split from src/object/core.rs — object_id helper for PyObjectRef::get_id.
use std::cell::{Cell, RefCell};
use std::collections::HashMap;

const TAG_NONE: usize = 0x10 << 56;
const TAG_BOOL: usize = 0x11 << 56;
const TAG_INT: usize = 0x12 << 56;
const TAG_FLOAT: usize = 0x13 << 56;

pub(crate) fn none_id() -> usize { TAG_NONE }
pub(crate) fn bool_id(b: bool) -> usize { TAG_BOOL | (b as usize) }
pub(crate) fn int_id(n: i64) -> usize { TAG_INT | ((n as u64 as usize) & 0x00ff_ffff_ffff_ffff) }
pub(crate) fn float_id(bits: u64) -> usize { TAG_FLOAT | ((bits as usize) & 0x00ff_ffff_ffff_ffff) }

thread_local! {
    static NEXT_HEAP_ID: Cell<usize> = const { Cell::new(1) };
    static HEAP_ID_TABLE: RefCell<HashMap<usize, usize>> = RefCell::new(HashMap::new());
}

pub(crate) fn heap_id(addr: usize) -> usize {
    HEAP_ID_TABLE.with(|t| {
        if let Some(&id) = t.borrow().get(&addr) { return id; }
        let id = NEXT_HEAP_ID.with(|c| { let v = c.get(); c.set(v+1); v });
        t.borrow_mut().insert(addr, id);
        id
    })
}
