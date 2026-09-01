// Split from src/modules/time/datetime.rs — isoformat helpers.
use crate::object::*;
use crate::modules::time::{format_offset_iso, get_utcoffset_seconds, inst_get_i64};
use super::{datetime_ordinal, datetime_tzinfo};

pub(crate) fn datetime_isoformat(obj: &PyObjectRef, sep: char) -> String {
    datetime_isoformat_ts(obj, sep, None)
}

/// timespec: None => auto (omit fraction when us==0); Some(n) => emit exactly
/// n fractional digits (0,3,6 are the meaningful ones; others truncated).
pub(crate) fn datetime_isoformat_ts(obj: &PyObjectRef, sep: char, timespec: Option<usize>) -> String {
    let y = inst_get_i64(obj, "year");
    let mo = inst_get_i64(obj, "month");
    let d = inst_get_i64(obj, "day");
    let h = inst_get_i64(obj, "hour");
    let mi = inst_get_i64(obj, "minute");
    let s = inst_get_i64(obj, "second");
    let us = inst_get_i64(obj, "microsecond");
    // Shared timezone suffix builder for every early-return path below.
    fn tz_suffix(obj: &PyObjectRef, h: i64, mi: i64, s: i64) -> String {
        let tz = datetime_tzinfo(obj);
        if let Some(off) =
            get_utcoffset_seconds(&tz, datetime_ordinal(obj), h * 3600 + mi * 60 + s)
        {
            format_offset_iso(off)
        } else {
            String::new()
        }
    }
    let tzs = tz_suffix(obj, h, mi, s);
    // timespec codes: 1=hours 2=minutes 3=seconds 4=milliseconds 5=micros
    // 0=auto (fraction only when nonzero). Hours/minutes drop seconds.
    match timespec {
        Some(1) => {
            return format!("{:04}-{:02}-{:02}{}{:02}{}", y, mo, d, sep, h, tzs);
        }
        Some(2) => {
            return format!("{:04}-{:02}-{:02}{}{:02}:{:02}{}", y, mo, d, sep, h, mi, tzs);
        }
        Some(3) => {
            return format!(
                "{:04}-{:02}-{:02}{}{:02}:{:02}:{:02}.000{}",
                y, mo, d, sep, h, mi, s, tzs
            );
        }
        Some(4) => {
            return format!(
                "{:04}-{:02}-{:02}{}{:02}:{:02}:{:02}.{:03}{}",
                y,
                mo,
                d,
                sep,
                h,
                mi,
                s,
                us / 1000,
                tzs
            );
        }
        _ => {}
    }
    let mut out = format!("{:04}-{:02}-{:02}{}{:02}:{:02}:{:02}", y, mo, d, sep, h, mi, s);
    if timespec == Some(5) || us != 0 {
        out.push_str(&format!(".{:06}", us));
    }
    let tz = datetime_tzinfo(obj);
    if let Some(off) = get_utcoffset_seconds(&tz, datetime_ordinal(obj), h * 3600 + mi * 60 + s) {
        out.push_str(&format_offset_iso(off));
    }
    out
}
