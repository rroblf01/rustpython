use crate::object::*;
use std::collections::HashMap;

pub fn create_random_cmodule_dict() -> HashMap<String, PyObjectRef> {
    // Delegates to the faithful MT19937 implementation in rand.rs --
    // replaces the old LCG stub that backed Lib/random.py's pure-Python
    // generator (getrandbits(2**31) took effectively forever there).
    crate::modules::rand::create_random_dict()
}
