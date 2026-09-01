// Split from src/object/core.rs — TypeDict / DictMap / helpers.
use crate::interner::{self, StrId};
use super::hasher::FxBuildHasher;
use crate::object::PyObjectRef;
use std::collections::HashMap;

pub type TypeDict = HashMap<StrId, PyObjectRef, FxBuildHasher>;

pub trait DictMap {
    fn get_str(&self, name: &str) -> Option<&PyObjectRef>;
    fn insert_str(&mut self, name: &str, val: PyObjectRef) -> Option<PyObjectRef>;
    fn contains_key_str(&self, name: &str) -> bool;
}
impl DictMap for HashMap<String, PyObjectRef> {
    fn get_str(&self, name: &str) -> Option<&PyObjectRef> { self.get(name) }
    fn insert_str(&mut self, name: &str, val: PyObjectRef) -> Option<PyObjectRef> { self.insert(name.to_string(), val) }
    fn contains_key_str(&self, name: &str) -> bool { self.contains_key(name) }
}
impl<S: std::hash::BuildHasher> DictMap for HashMap<StrId, PyObjectRef, S> {
    fn get_str(&self, name: &str) -> Option<&PyObjectRef> { self.get(&interner::intern(name)) }
    fn insert_str(&mut self, name: &str, val: PyObjectRef) -> Option<PyObjectRef> { self.insert(interner::intern(name), val) }
    fn contains_key_str(&self, name: &str) -> bool { self.contains_key(&interner::intern(name)) }
}

pub(crate) fn str_map_to_strid_map<V>(map: HashMap<String, V>) -> HashMap<StrId, V> {
    map.into_iter().map(|(k, v)| (interner::intern(&k), v)).collect()
}
pub(crate) fn str_map_to_typedict<V>(map: HashMap<String, V>) -> HashMap<StrId, V, FxBuildHasher> {
    map.into_iter().map(|(k, v)| (interner::intern(&k), v)).collect()
}
