use std::collections::HashMap;

pub(crate) struct ParsedTz {
    pub(crate) transitions: Vec<i64>,
    pub(crate) trans_types: Vec<u8>,
    pub(crate) ttinfos: Vec<(i32, bool, String)>, // (utc offset seconds, is_dst, designation)
}

fn read_i32_be(data: &[u8], pos: usize) -> i32 {
    i32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]])
}

fn read_i64_be(data: &[u8], pos: usize) -> i64 {
    let mut b = [0u8; 8];
    b.copy_from_slice(&data[pos..pos + 8]);
    i64::from_be_bytes(b)
}

fn parse_tzif_block(data: &[u8], pos: usize, time_size: usize) -> Option<(ParsedTz, usize)> {
    if pos + 44 > data.len() {
        return None;
    }
    let isutcnt = read_i32_be(data, pos + 20) as usize;
    let isstdcnt = read_i32_be(data, pos + 24) as usize;
    let leapcnt = read_i32_be(data, pos + 28) as usize;
    let timecnt = read_i32_be(data, pos + 32) as usize;
    let typecnt = read_i32_be(data, pos + 36) as usize;
    let charcnt = read_i32_be(data, pos + 40) as usize;
    let mut p = pos + 44;

    let mut transitions = Vec::with_capacity(timecnt);
    for _ in 0..timecnt {
        if p + time_size > data.len() {
            return None;
        }
        let t = if time_size == 8 {
            read_i64_be(data, p)
        } else {
            read_i32_be(data, p) as i64
        };
        transitions.push(t);
        p += time_size;
    }
    let mut trans_types = Vec::with_capacity(timecnt);
    for _ in 0..timecnt {
        trans_types.push(*data.get(p)?);
        p += 1;
    }
    let mut ttinfo_raw = Vec::with_capacity(typecnt);
    for _ in 0..typecnt {
        if p + 6 > data.len() {
            return None;
        }
        let utoff = read_i32_be(data, p);
        let isdst = data[p + 4] != 0;
        let desigidx = data[p + 5] as usize;
        ttinfo_raw.push((utoff, isdst, desigidx));
        p += 6;
    }
    if p + charcnt > data.len() {
        return None;
    }
    let charpool = &data[p..p + charcnt];
    p += charcnt;
    p += leapcnt * (time_size + 4);
    p += isstdcnt;
    p += isutcnt;

    let ttinfos: Vec<(i32, bool, String)> = ttinfo_raw
        .into_iter()
        .map(|(utoff, isdst, idx)| {
            let desig = if idx < charpool.len() {
                let end = charpool[idx..]
                    .iter()
                    .position(|&b| b == 0)
                    .map(|o| idx + o)
                    .unwrap_or(charpool.len());
                String::from_utf8_lossy(&charpool[idx..end]).to_string()
            } else {
                String::new()
            };
            (utoff, isdst, desig)
        })
        .collect();

    Some((
        ParsedTz {
            transitions,
            trans_types,
            ttinfos,
        },
        p,
    ))
}

fn parse_tzif(bytes: &[u8]) -> Option<ParsedTz> {
    if bytes.len() < 44 || &bytes[0..4] != b"TZif" {
        return None;
    }
    let version = bytes[4];
    let (v1_result, next_pos) = parse_tzif_block(bytes, 0, 4)?;
    if version == 0 {
        return Some(v1_result);
    }
    if next_pos + 4 <= bytes.len() && &bytes[next_pos..next_pos + 4] == b"TZif" {
        if let Some((v2_result, _)) = parse_tzif_block(bytes, next_pos, 8) {
            return Some(v2_result);
        }
    }
    Some(v1_result)
}

pub(crate) fn tz_offset_for_instant(tz: &ParsedTz, instant: i64) -> (i32, bool, String) {
    if tz.ttinfos.is_empty() {
        return (0, false, "UTC".to_string());
    }
    if tz.transitions.is_empty() {
        return tz.ttinfos[0].clone();
    }
    let idx = match tz.transitions.binary_search(&instant) {
        Ok(i) => Some(i),
        Err(0) => None,
        Err(i) => Some(i - 1),
    };
    match idx {
        Some(i) => tz.ttinfos[tz.trans_types[i] as usize].clone(),
        None => tz
            .ttinfos
            .iter()
            .find(|t| !t.1)
            .cloned()
            .unwrap_or_else(|| tz.ttinfos[0].clone()),
    }
}

thread_local! {
    static TZ_CACHE: std::cell::RefCell<HashMap<String, std::rc::Rc<ParsedTz>>> = std::cell::RefCell::new(HashMap::new());
}

/// Loads and caches a real IANA time zone from the system's tzdata
/// (`/usr/share/zoneinfo`). Rejects keys that could escape the zoneinfo
/// root (defense against path traversal via a crafted zone key).
pub(crate) fn load_tz(key: &str) -> Option<std::rc::Rc<ParsedTz>> {
    if key.is_empty() || key.contains("..") || key.starts_with('/') || key.contains('\0') {
        return None;
    }
    TZ_CACHE.with(|c| {
        if let Some(v) = c.borrow().get(key) {
            return Some(v.clone());
        }
        let path = format!("/usr/share/zoneinfo/{}", key);
        let bytes = std::fs::read(&path).ok()?;
        let parsed = parse_tzif(&bytes)?;
        let rc = std::rc::Rc::new(parsed);
        c.borrow_mut().insert(key.to_string(), rc.clone());
        Some(rc)
    })
}
