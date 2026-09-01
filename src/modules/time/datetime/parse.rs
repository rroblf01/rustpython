// Split from src/modules/time/datetime.rs — isoformat parsing helpers.
use crate::object::*;
use crate::modules::time::helpers::{ordinal_to_ymd, ymd_to_ordinal};
use crate::modules::time::{get_utc_singleton, make_timezone};
use super::make_datetime;

pub(crate) fn parse_datetime_isoformat(s: &str) -> PyResult<PyObjectRef> {
    // Full CPython-3.11-style parser: same grammar as the date-type
    // fromisoformat above (basic/extended dates, ISO weeks, ordinal dates,
    // ANY single-char separator incl. multi-byte ones, fractional seconds
    // with ',' or '.', truncated to microseconds, 'Z'/+-HH[:MM[:SS]] zones).
    let bad = || {
        if std::env::var("RPY_ISO_LOG").is_ok() { eprintln!("DTI-REJ {:?}", s); }
        PyError::value_error("Invalid isoformat string")
    };
    // Accept ASCII/UTF-8 bytes like the date-type parser does.
    let owned;
    let s_str: &str = if let Some(stripped) = s.strip_prefix("b'").and_then(|x| x.strip_suffix('\'')) {
        owned = stripped.to_string();
        &owned
    } else {
        s
    };
    let mut t = s_str.trim();
    if t.is_empty() {
        return Err(bad());
    }
    let mut tz_off: Option<i64> = None;
    if t.ends_with(['Z', 'z']) {
        tz_off = Some(0);
        t = &t[..t.len() - 1];
    } else if let Some(pos) = t.rfind(['+', '-']) {
        // A '+'/'-' sitting EXACTLY at the date/time boundary is the
        // SEPARATOR (test_fromisoformat_ambiguous): '2018-01-31+12:15' means
        // 12:15 as the TIME, not a +12:15 zone. Only positions past the
        // boundary start a real zone.
        let boundary_is_sep = pos == 10 || pos == 8;
        if !boundary_is_sep && pos > 8 {
            let zone = &t[pos..];
            let zc: String = zone.chars().filter(|c| *c != ':').collect();
            let neg = zc.starts_with('-');
            let zbody = &zc[1..];
            let digits_ok = !zbody.is_empty() && zbody.chars().all(|c| c.is_ascii_digit());
            if digits_ok && (zbody.len() == 2 || zbody.len() == 4 || zbody.len() == 6) {
                let hh: i64 = zbody[0..2].parse().unwrap_or(99);
                let mm: i64 = if zbody.len() >= 4 {
                    zbody[2..4].parse().unwrap_or(99)
                } else {
                    0
                };
                let ss: i64 = if zbody.len() == 6 {
                    zbody[4..6].parse().unwrap_or(99)
                } else {
                    0
                };
                if hh <= 23 && mm < 60 && ss < 60 {
                    let total = hh * 3600 + mm * 60 + ss;
                    tz_off = Some(if neg { -total } else { total });
                    t = &t[..pos];
                }
            }
        }
    }
    // Split date / time at ANY single non-digit char after index 8.
    fn parse_date(ds: &str) -> Option<(i64, i64, i64)> {
        let ds = ds.trim();
        if ds.len() == 10 && ds.as_bytes()[4] == b'-' && ds.as_bytes()[7] == b'-' {
            return Some((
                ds.get(0..4)?.parse().ok()?,
                ds.get(5..7)?.parse().ok()?,
                ds.get(8..10)?.parse().ok()?,
            ));
        }
        let compact: String = ds.chars().filter(|&c| c != '-').collect();
        if compact.len() == 8 && compact.chars().all(|c| c.is_ascii_digit()) {
            return Some((
                compact.get(0..4)?.parse().ok()?,
                compact.get(4..6)?.parse().ok()?,
                compact.get(6..8)?.parse().ok()?,
            ));
        }
        if compact.len() == 7 && compact.chars().all(|c| c.is_ascii_digit())
            && !compact.contains(['W', 'w'])
        {
            let y: i64 = compact.get(0..4)?.parse().ok()?;
            let doy: i64 = compact.get(4..7)?.parse().ok()?;
            if !(1..=366).contains(&doy) {
                return None;
            }
            let jan1 = ymd_to_ordinal(y, 1, 1);
            return Some(ordinal_to_ymd(jan1 + doy - 1));
        }
        if let Some(wp) = compact.find(|c: char| c == 'W' || c == 'w') {
            if wp == 4 && compact.len() >= 7 && compact.len() <= 9 {
                let y: i64 = compact.get(0..4)?.parse().ok()?;
                let wk: i64 = compact.get(5..7)?.parse().ok()?;
                let wd: i64 = if compact.len() >= 8 {
                    compact.get(7..8)?.parse().ok()?
                } else {
                    1
                };
                if !(1..=53).contains(&wk) || !(1..=7).contains(&wd) {
                    return None;
                }
                let jan4_ord = ymd_to_ordinal(y, 1, 4);
                let jan4_wd = ((jan4_ord % 7) + 6) % 7;
                let ord = jan4_ord - jan4_wd + (wk - 1) * 7 + (wd - 1);
                return Some(ordinal_to_ymd(ord));
            }
        }
        None
    }

    // ISO-week BASIC with trailing time ('2026W01516' = 2026-W01-5 16:00):
    // fixed-width fields -- YYYY W ww [D] [HH[MM[SS]]]. Handled before the
    // generic separator scan because every position here is a digit.
    {
        let wb = t.as_bytes();
        if t.len() >= 7
            && wb[4] == b'W'
            && wb[..4].iter().all(|b| b.is_ascii_digit())
            && wb[5..7].iter().all(|b| b.is_ascii_digit())
            && !t.contains('-')
            && !t.contains('+')
        {
            let mut day_end = 7;
            let mut day = 1i64;
            if t.len() > 7 && wb[7].is_ascii_digit() {
                day = (wb[7] - b'0') as i64;
                day_end = 8;
            }
            let time_part = &t[day_end.min(t.len())..];
            let hour_ok = time_part.len() % 2 == 0;
            if !(1..=7).contains(&day)
                && !(time_part.is_empty() || (hour_ok && !time_part.is_empty()))
            {
                return Err(bad());
            }
            let ds = format!("{}W{:02}{}", &t[..4], &t[5..7], day);
            if let Some((yy, mm_, dd_)) = (|| {
                let y_: i64 = ds.get(0..4)?.parse().ok()?;
                let wk: i64 = ds.get(5..7)?.parse().ok()?;
                if !(1..=53).contains(&wk) {
                    return None;
                }
                let jan4_ord = ymd_to_ordinal(y_, 1, 4);
                let jan4_wd = ((jan4_ord % 7) + 6) % 7;
                Some(ordinal_to_ymd(jan4_ord - jan4_wd + (wk - 1) * 7 + (day - 1)))
            })() {
                let (mut hh, mut mi, mut ss, mut us) = (0i64, 0i64, 0i64, 0i64);
                if !time_part.is_empty() {
                    if !time_part.chars().all(|c| c.is_ascii_digit()) {
                        return Err(bad());
                    }
                    match time_part.len() {
                        2 => hh = time_part.parse().unwrap_or(99),
                        4 => {
                            hh = time_part.get(0..2).and_then(|x| x.parse().ok()).unwrap_or(99);
                            mi = time_part.get(2..4).and_then(|x| x.parse().ok()).unwrap_or(99);
                        }
                        6 => {
                            hh = time_part.get(0..2).and_then(|x| x.parse().ok()).unwrap_or(99);
                            mi = time_part.get(2..4).and_then(|x| x.parse().ok()).unwrap_or(99);
                            ss = time_part.get(4..6).and_then(|x| x.parse().ok()).unwrap_or(99);
                        }
                        _ => return Err(bad()),
                    }
                    if hh > 23 || mi > 59 || ss > 59 {
                        return Err(bad());
                    }
                    let _ = us;
                }
                if let Some(off) = tz_off {
                    let tz = if off == 0 {
                        get_utc_singleton()
                    } else {
                        make_timezone(off, None)
                    };
                    return Ok(make_datetime(yy, mm_, dd_, hh, mi, ss, us, tz, 0));
                }
                return Ok(make_datetime(yy, mm_, dd_, hh, mi, ss, us, py_none(), 0));
            }
        }
    }

    // Separator discovery:
    //  a) '+'/'-' EXACTLY at the date/time boundary is the separator
    //     (test_fromisoformat_ambiguous: '2018-01-31+12:15' -> time 12:15).
    //  b) otherwise the first NON-DIGIT at byte index >= 8 that is not a
    //     '+'/'-' (those belong to zones or to extended week/ordinal dates
    //     like '2026-W01-3'); with backtracking if the resulting date part
    //     does not validate.
    fn time_like(rest: &str) -> bool {
        let core = match rest.find(['.', ',']) {
            Some(k) => &rest[..k],
            None => rest,
        };
        let dgs: String = core.chars().filter(|&c| c != ':').collect();
        if dgs.is_empty() || !dgs.chars().all(|c| c.is_ascii_digit()) {
            return false;
        }
        if !matches!(dgs.len(), 2 | 4 | 6) {
            return false;
        }
        let hh: i64 = dgs.get(0..2).and_then(|x| x.parse().ok()).unwrap_or(99);
        let mi: i64 = if dgs.len() >= 4 {
            dgs.get(2..4).and_then(|x| x.parse().ok()).unwrap_or(99)
        } else {
            0
        };
        let ss: i64 = if dgs.len() == 6 {
            dgs.get(4..6).and_then(|x| x.parse().ok()).unwrap_or(99)
        } else {
            0
        };
        hh <= 23 && mi < 60 && ss < 60
    }
    let (date_s, time_s) = if let Some(sep_pos) =
        t.rfind(['+', '-'])
            .filter(|&p| (p == 8 || p == 10) && time_like(&t[p + 1..]))
    {
        (&t[..sep_pos], &t[sep_pos + 1..])
    } else {
        match t
            .char_indices()
            .find(|(i, ch)| *i >= 8 && !ch.is_ascii_digit() && !matches!(ch, '+' | '-'))
        {
            Some((i, ch)) => {
                let ds = &t[..i];
                if parse_date(ds).is_some() {
                    (&t[..i], &t[i + ch.len_utf8()..])
                } else {
                    (t, "")
                }
            }
            None => (t, ""),
        }
    };
    let (y, mo, d) = match parse_date(date_s) {
        Some(v) => v,
        None => return Err(bad()),
    };
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) {
        return Err(bad());
    }
    let (mut hh, mut mi, mut ss, mut us) = (0i64, 0i64, 0i64, 0i64);
    if !time_s.is_empty() {
        let tp = time_s;
        let (tp, frac) = match tp.find(['.', ',']) {
            Some(dot) => (&tp[..dot], &tp[dot + 1..]),
            None => (tp, ""),
        };
        let digits: String = tp.chars().filter(|&c| c != ':').collect();
        if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
            return Err(bad());
        }
        match digits.len() {
            2 => hh = digits.parse().unwrap_or(99),
            4 => {
                hh = digits.get(0..2).and_then(|x| x.parse().ok()).unwrap_or(99);
                mi = digits.get(2..4).and_then(|x| x.parse().ok()).unwrap_or(99);
            }
            6 => {
                hh = digits.get(0..2).and_then(|x| x.parse().ok()).unwrap_or(99);
                mi = digits.get(2..4).and_then(|x| x.parse().ok()).unwrap_or(99);
                ss = digits.get(4..6).and_then(|x| x.parse().ok()).unwrap_or(99);
            }
            _ => return Err(bad()),
        }
        if hh > 23 || mi > 59 || ss > 59 {
            return Err(bad());
        }
        if !frac.is_empty() {
            if !frac.chars().all(|c| c.is_ascii_digit()) {
                return Err(bad());
            }
            let f6: String = frac.chars().take(6).collect();
            us = format!("{:<06}", f6).parse().unwrap_or(0);
        }
    }
    if let Some(off) = tz_off {
        let tz = if off == 0 {
            get_utc_singleton()
        } else {
            make_timezone(off, None)
        };
        return Ok(make_datetime(y, mo, d, hh, mi, ss, us, tz, 0));
    }
    Ok(make_datetime(y, mo, d, hh, mi, ss, us, py_none(), 0))
}

pub(crate) fn parse_date_part(date_part: &str) -> PyResult<(i64, i64, i64)> {
    // Try YYYY-MM-DD format
    let parts: Vec<&str> = date_part.splitn(3, '-').collect();
    if parts.len() == 3
        && parts[0].chars().all(|c| c.is_ascii_digit())
        && parts[1].chars().all(|c| c.is_ascii_digit())
        && parts[2].chars().all(|c| c.is_ascii_digit())
    {
        let y: i64 = parts[0]
            .parse()
            .map_err(|_| PyError::value_error("Invalid isoformat string"))?;
        let m: i64 = parts[1]
            .parse()
            .map_err(|_| PyError::value_error("Invalid isoformat string"))?;
        let d: i64 = parts[2]
            .parse()
            .map_err(|_| PyError::value_error("Invalid isoformat string"))?;
        return Ok((y, m, d));
    }
    // Try YYYYMMDD format (8 digits)
    if date_part.len() == 8 && date_part.chars().all(|c| c.is_ascii_digit()) {
        let y: i64 = date_part[..4]
            .parse()
            .map_err(|_| PyError::value_error("Invalid isoformat string"))?;
        let m: i64 = date_part[4..6]
            .parse()
            .map_err(|_| PyError::value_error("Invalid isoformat string"))?;
        let d: i64 = date_part[6..8]
            .parse()
            .map_err(|_| PyError::value_error("Invalid isoformat string"))?;
        return Ok((y, m, d));
    }
    // Try ISO week format: YYYYWww, YYYY-Www, YYYYWwwd, YYYY-Www-d
    let compact: String = date_part.chars().filter(|c| *c != '-').collect();
    if compact.len() >= 6 && compact.len() <= 8 {
        let w_pos = compact.find(|c: char| c == 'W' || c == 'w');
        if let Some(wp) = w_pos {
            if wp == 4 && compact.len() >= 6 {
                if let (Ok(y), Ok(wk)) = (compact[..4].parse::<i64>(), compact[5..7].parse::<i64>())
                {
                    let wd: i64 = if compact.len() >= 8 {
                        compact[7..8].parse().unwrap_or(1)
                    } else {
                        1
                    };
                    if y >= 1 && y <= 9999 && wk >= 1 && wk <= 53 && wd >= 1 && wd <= 7 {
                        let jan4_ord = ymd_to_ordinal(y, 1, 4);
                        let jan4_weekday = ((jan4_ord % 7) + 6) % 7;
                        let week1_monday = jan4_ord - jan4_weekday;
                        let target_ord = week1_monday + (wk - 1) * 7 + (wd - 1);
                        return Ok(ordinal_to_ymd(target_ord));
                    }
                }
            }
        }
    }
    Err(PyError::value_error("Invalid isoformat string"))
}

pub(crate) fn parse_time_part(time_part: &str) -> PyResult<(i64, i64, i64, i64)> {
    if time_part.is_empty() {
        return Ok((0, 0, 0, 0));
    }
    // Compact time format (no colons): HH, HHMM, HHMMSS, or with fractional
    if !time_part.contains(':') {
        let (int_part, frac_part) =
            if let Some(pos) = time_part.find(|c: char| c == '.' || c == ',') {
                if pos + 1 >= time_part.len() {
                    return Err(PyError::value_error("Invalid isoformat string"));
                }
                (&time_part[..pos], Some(&time_part[pos + 1..]))
            } else {
                (time_part, None)
            };
        // Validate int_part is all digits
        if !int_part.chars().all(|c| c.is_ascii_digit()) {
            return Err(PyError::value_error("Invalid isoformat string"));
        }
        let (h, m, s) = match int_part.len() {
            1 | 2 => (int_part.parse().unwrap_or(0), 0, 0),
            4 => (
                int_part[..2].parse().unwrap_or(0),
                int_part[2..].parse().unwrap_or(0),
                0,
            ),
            6 => (
                int_part[..2].parse().unwrap_or(0),
                int_part[2..4].parse().unwrap_or(0),
                int_part[4..].parse().unwrap_or(0),
            ),
            _ => {
                return Err(PyError::value_error("Invalid isoformat string"));
            }
        };
        let us = match frac_part {
            Some(frac) => {
                let padded = format!("{:0<6}", frac);
                padded[..6.min(padded.len())].parse().unwrap_or(0)
            }
            None => 0,
        };
        return Ok((h, m, s, us));
    }
    // Colon-separated: HH:MM, HH:MM:SS, HH:MM:SS.fff, HH:MM:SS,fff
    // Reject trailing colons
    if time_part.ends_with(':') {
        return Err(PyError::value_error("Invalid isoformat string"));
    }
    let tparts: Vec<&str> = time_part.splitn(3, ':').collect();
    // Validate each part is non-empty and digits (or has fractional)
    for (i, part) in tparts.iter().enumerate() {
        if part.is_empty() {
            return Err(PyError::value_error("Invalid isoformat string"));
        }
        if i < 2 {
            // hour and minute must be all digits
            if !part.chars().all(|c| c.is_ascii_digit()) {
                return Err(PyError::value_error("Invalid isoformat string"));
            }
        } else {
            // second part: digits possibly followed by . or , and fraction
            let clean: String = part.chars().filter(|c| c.is_ascii_digit()).collect();
            if clean.is_empty() {
                return Err(PyError::value_error("Invalid isoformat string"));
            }
        }
    }
    let hour: i64 = tparts.first().and_then(|v| v.parse().ok()).unwrap_or(0);
    let minute: i64 = tparts.get(1).and_then(|v| v.parse().ok()).unwrap_or(0);
    let (second, micro): (i64, i64) = match tparts.get(2) {
        Some(sec_str) => {
            let frac_pos = sec_str.find('.').or_else(|| sec_str.find(','));
            match frac_pos {
                Some(dotpos) => {
                    if dotpos + 1 >= sec_str.len() {
                        return Err(PyError::value_error("Invalid isoformat string"));
                    }
                    let sec: i64 = sec_str[..dotpos].parse().unwrap_or(0);
                    let frac = &sec_str[dotpos + 1..];
                    let padded = format!("{:0<6}", frac);
                    let us: i64 = padded[..6.min(padded.len())].parse().unwrap_or(0);
                    (sec, us)
                }
                None => (sec_str.parse().unwrap_or(0), 0),
            }
        }
        None => (0, 0),
    };
    Ok((hour, minute, second, micro))
}
