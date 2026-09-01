use crate::object::*;
use std::collections::HashMap;

pub fn create_gettext_dict() -> HashMap<String, PyObjectRef> {
    HashMap::new()
}

/// gettext module source — see VirtualMachine::install_source_defined_stdlib.
pub const GETTEXT_SOURCE: &str = include_str!("../gettext_extra.py");
