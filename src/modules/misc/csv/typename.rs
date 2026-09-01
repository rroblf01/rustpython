use crate::object::*;

pub fn type_name_of(obj: &PyObjectRef) -> String {
    let b = obj.borrow();
    b.type_name()
}
