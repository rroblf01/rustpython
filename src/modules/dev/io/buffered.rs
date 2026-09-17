use crate::buffered_class;
use crate::modules::dev::io::fixup_self_mro;
use crate::object::*;
use std::collections::HashMap;

/// Register the four buffered IO wrapper types that delegate to an
/// underlying raw object via `buffered_class!`.
pub fn register_buffered_classes(d: &mut HashMap<String, PyObjectRef>, buf_cls: &PyObjectRef) {
    let br_cls = buffered_class!("BufferedReader", buf_cls);
    fixup_self_mro(&br_cls);
    d.insert_str("BufferedReader", br_cls.clone());
    let bw_cls = buffered_class!("BufferedWriter", buf_cls);
    fixup_self_mro(&bw_cls);
    d.insert_str("BufferedWriter", bw_cls.clone());
    let brp_cls = buffered_class!("BufferedRWPair", buf_cls);
    fixup_self_mro(&brp_cls);
    d.insert_str("BufferedRWPair", brp_cls.clone());
    let brnd_cls = buffered_class!("BufferedRandom", buf_cls);
    fixup_self_mro(&brnd_cls);
    d.insert_str("BufferedRandom", brnd_cls.clone());
}
