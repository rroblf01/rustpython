use crate::buffered_class;
use crate::object::*;
use std::collections::HashMap;

/// Register the four buffered IO wrapper types that delegate to an
/// underlying raw object via `buffered_class!`.
pub fn register_buffered_classes(d: &mut HashMap<String, PyObjectRef>, buf_cls: &PyObjectRef) {
    let br_cls = buffered_class!("BufferedReader", buf_cls);
    d.insert_str("BufferedReader", br_cls.clone());
    let bw_cls = buffered_class!("BufferedWriter", buf_cls);
    d.insert_str("BufferedWriter", bw_cls.clone());
    let brp_cls = buffered_class!("BufferedRWPair", buf_cls);
    d.insert_str("BufferedRWPair", brp_cls.clone());
    let brnd_cls = buffered_class!("BufferedRandom", buf_cls);
    d.insert_str("BufferedRandom", brnd_cls.clone());
}
