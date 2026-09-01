pub(crate) const EPOCH_ORDINAL: i64 = 719163; // ymd_to_ordinal(1970, 1, 1)

pub(crate) fn is_leap_year(year: i64) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

const DAYS_IN_MONTH: [i64; 13] = [0, 31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];

pub(crate) fn days_in_month(year: i64, month: i64) -> i64 {
    if month == 2 && is_leap_year(year) {
        29
    } else {
        DAYS_IN_MONTH[month as usize]
    }
}

pub(crate) fn days_before_month_table() -> [i64; 13] {
    let mut t = [0i64; 13];
    let mut acc = 0;
    for m in 1..13 {
        t[m] = acc;
        acc += DAYS_IN_MONTH[m];
    }
    t
}

pub(crate) fn days_before_year(year: i64) -> i64 {
    let y = year - 1;
    y * 365 + y / 4 - y / 100 + y / 400
}

pub(crate) fn days_before_month(year: i64, month: i64) -> i64 {
    days_before_month_table()[month as usize]
        + if month > 2 && is_leap_year(year) {
            1
        } else {
            0
        }
}

pub(crate) fn ymd_to_ordinal(year: i64, month: i64, day: i64) -> i64 {
    days_before_year(year) + days_before_month(year, month) + day
}

/// Port of CPython's `datetime._ord2ymd` (proleptic Gregorian calendar).
pub(crate) fn ordinal_to_ymd(n_in: i64) -> (i64, i64, i64) {
    let t = days_before_month_table();
    let mut n = n_in - 1;
    let n400 = n.div_euclid(146097);
    n = n.rem_euclid(146097);
    let mut year = n400 * 400 + 1;
    let n100 = n.div_euclid(36524);
    n = n.rem_euclid(36524);
    let n4 = n.div_euclid(1461);
    n = n.rem_euclid(1461);
    let n1 = n.div_euclid(365);
    n = n.rem_euclid(365);
    year += n100 * 100 + n4 * 4 + n1;
    if n1 == 4 || n100 == 4 {
        return (year - 1, 12, 31);
    }
    let mut month = (n + 50) >> 5;
    let mut preceding = t[month as usize]
        + if month > 2 && is_leap_year(year) {
            1
        } else {
            0
        };
    if preceding > n {
        month -= 1;
        preceding -= days_in_month(year, month);
    }
    n -= preceding;
    (year, month, n + 1)
}

/// 0 = Monday .. 6 = Sunday, matching CPython's `date.weekday()`.
pub(crate) fn weekday_from_ordinal(ord: i64) -> i64 {
    (ord + 6).rem_euclid(7)
}

pub(crate) fn day_of_year(year: i64, ordinal: i64) -> i64 {
    ordinal - days_before_year(year) - 1
}

pub(crate) fn normalize_timedelta(days: i64, seconds: i64, microseconds: i64) -> (i64, i64, i64) {
    let extra_s = microseconds.div_euclid(1_000_000);
    let microseconds = microseconds.rem_euclid(1_000_000);
    let seconds = seconds + extra_s;
    let extra_d = seconds.div_euclid(86400);
    let seconds = seconds.rem_euclid(86400);
    let days = days + extra_d;
    (days, seconds, microseconds)
}
