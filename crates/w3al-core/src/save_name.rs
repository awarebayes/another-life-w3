//! Names of saves the way the game makes them: `ManualSave_<a>_<b>_<c>`.
//!
//! The Load list shows the quest name only for files with this shape (user-reported in game,
//! docs/research.md 2026-11-04). What the three hex fields are, from 147 real saves
//! (`docs/findings/save-name-ids.md`):
//!
//! * `b`, `c`: the local date and time of the save, packed. `b` = `year << 20 | (month-1) << 15 |
//!   (day-1) << 10` (low 10 bits always 0), `c` = `hour << 22 | minute << 16 | second << 10 |
//!   millisecond`. They equal the high and low half of the last `time` entry of the payload's
//!   `saveInfo`, a few milliseconds earlier. OBSERVED on every real save we have.
//! * `a`: an opaque key that is the same for the same tracked quest (`JTrackedQuest` GUID)
//!   across players and years. We cannot compute it, so a new name keeps the template's.
//!
//! The game accepted names whose `b`/`c` do not match the payload (invented ids, 2026-11-04), so
//! the stamp only has to look like a plausible, unique, later time.
//!
//! HYPOTHESIS: the stamp is made from UTC, not local time (std has no local offset). The game
//! uses local time; whether the Load list orders by this stamp, by the payload, or by the file's
//! modified time is untested (see the findings file).

/// Prefix of every name we generate. `CheckPoint_`/`QuickSave_` files carry the same ids but
/// the Load list is known to label `ManualSave_` ones only.
pub const PREFIX: &str = "ManualSave";

/// `a` for a template whose own name has no `<a>_<b>_<c>` ids (a renamed copy). HYPOTHESIS: any
/// real value loads; this one is the tracked-quest key of the opening of the game (25 saves of
/// one playthrough carry it). It may show the wrong quest name in the Load list.
pub const DEFAULT_QUEST_KEY: &str = "8559a";

/// Calendar time as the game packs it into `b` and `c`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct SaveTime {
    pub year: u32,
    /// 1..=12
    pub month: u32,
    /// 1..=31
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
    pub millis: u32,
}

impl SaveTime {
    /// The packed `b` field.
    pub fn b(&self) -> u32 {
        self.year << 20 | (self.month - 1) << 15 | (self.day - 1) << 10
    }

    /// The packed `c` field.
    pub fn c(&self) -> u32 {
        self.hour << 22 | self.minute << 16 | self.second << 10 | self.millis
    }

    /// Unpack a `b`/`c` pair. `None` when it is not a plausible date (year before 2000, a month
    /// above 12, a day the month does not have, a time of day that does not exist, or non-zero
    /// low bits in `b`, which no real save has).
    pub fn from_bc(b: u32, c: u32) -> Option<Self> {
        let t = SaveTime {
            year: b >> 20,
            month: ((b >> 15) & 31) + 1,
            day: ((b >> 10) & 31) + 1,
            hour: c >> 22,
            minute: (c >> 16) & 63,
            second: (c >> 10) & 63,
            millis: c & 1023,
        };
        let ok = b & 1023 == 0
            && t.year >= 2000
            && t.month <= 12
            && t.day <= days_in_month(t.year, t.month)
            && t.hour < 24
            && t.minute < 60
            && t.second < 60
            && t.millis < 1000;
        ok.then_some(t)
    }

    /// Calendar time for milliseconds since 1970-01-01 00:00:00 (read as UTC).
    pub fn from_unix_millis(ms: i64) -> Self {
        let (y, mo, d) = civil_from_days(ms.div_euclid(86_400_000));
        let rem = ms.rem_euclid(86_400_000) as u32;
        SaveTime {
            year: y as u32,
            month: mo,
            day: d,
            hour: rem / 3_600_000,
            minute: rem / 60_000 % 60,
            second: rem / 1000 % 60,
            millis: rem % 1000,
        }
    }

    pub fn to_unix_millis(&self) -> i64 {
        days_from_civil(self.year as i64, self.month, self.day) * 86_400_000
            + ((self.hour as i64 * 60 + self.minute as i64) * 60 + self.second as i64) * 1000
            + self.millis as i64
    }

    pub fn plus_millis(&self, ms: i64) -> Self {
        Self::from_unix_millis(self.to_unix_millis() + ms)
    }

    /// The current time, read as UTC (see the module note).
    pub fn now_utc() -> Self {
        let ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as i64);
        Self::from_unix_millis(ms)
    }
}

/// A file stem of the shape `<Prefix>_<a>_<b>_<c>`: three lowercase hex fields (1 to 8 digits,
/// no padding) after an alphabetic prefix (`ManualSave`, `QuickSave`, `CheckPoint`, ...).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameStem {
    pub prefix: String,
    pub a: String,
    pub b: u32,
    pub c: u32,
}

pub fn parse_stem(stem: &str) -> Option<GameStem> {
    fn hex(s: &str) -> Option<u32> {
        let ok = !s.is_empty() && s.len() <= 8 && s.bytes().all(|c| matches!(c, b'0'..=b'9' | b'a'..=b'f'));
        ok.then(|| u32::from_str_radix(s, 16).ok()).flatten()
    }
    let mut it = stem.split('_');
    let (prefix, a, b, c) = (it.next()?, it.next()?, it.next()?, it.next()?);
    if it.next().is_some() || prefix.is_empty() || !prefix.bytes().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    hex(a)?;
    Some(GameStem { prefix: prefix.to_owned(), a: a.to_owned(), b: hex(b)?, c: hex(c)? })
}

/// `ManualSave_<a>_<b>_<c>` for a new save made from `template_stem` at `now`.
///
/// * `a` is the template's (same tracked quest) or [`DEFAULT_QUEST_KEY`].
/// * The stamp is `now`, but strictly later than the template's when the template's ids decode
///   to a date: a template from "the future" (a local clock ahead of UTC, a hand-made name)
///   gets its stamp plus one second.
/// * `taken(stem)` says whether a name is already in use; the stamp then moves on one
///   millisecond at a time until the name is free.
pub fn next_stem(template_stem: Option<&str>, now: SaveTime, mut taken: impl FnMut(&str) -> bool) -> String {
    let t = template_stem.and_then(parse_stem);
    let a = t.as_ref().map_or(DEFAULT_QUEST_KEY, |t| t.a.as_str());
    let template_time = t.as_ref().and_then(|t| SaveTime::from_bc(t.b, t.c));
    let mut when = match template_time {
        Some(f) if f >= now => f.plus_millis(1000),
        _ => now,
    };
    let mut stem = format_stem(a, &when);
    // Bounded only so a broken `taken` cannot hang the export; `create_new` is the real guard.
    for _ in 0..10_000 {
        if !taken(&stem) {
            break;
        }
        when = when.plus_millis(1);
        stem = format_stem(a, &when);
    }
    stem
}

fn format_stem(a: &str, when: &SaveTime) -> String {
    format!("{PREFIX}_{a}_{:x}_{:x}", when.b(), when.c())
}

fn days_in_month(year: u32, month: u32) -> u32 {
    match month {
        2 if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Days since 1970-01-01 -> (year, month, day). Howard Hinnant's civil-from-days.
pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + (m <= 2) as i64, m, d)
}

/// (year, month, day) -> days since 1970-01-01 (Hinnant's days-from-civil).
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = y - (m <= 2) as i64;
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(y: u32, mo: u32, d: u32, h: u32, mi: u32, s: u32, ms: u32) -> SaveTime {
        SaveTime { year: y, month: mo, day: d, hour: h, minute: mi, second: s, millis: ms }
    }

    #[test]
    fn civil_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(20_396), (2025, 11, 4));
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
        for z in [-1, 0, 59, 60, 11_016, 20_396, 25_000] {
            let (y, m, d) = civil_from_days(z);
            assert_eq!(days_from_civil(y, m, d), z);
        }
    }

    /// Real names, with the minute the user's own game saved them (file times, local +04:00):
    /// the packed ids decode to exactly that date and time of day.
    #[test]
    fn real_ids_decode_to_the_save_time() {
        // ManualSave_52586_7ea4c400_1030937: file written 2026-10-18 04:03:02 local.
        let t = SaveTime::from_bc(0x7ea4c400, 0x1030937).unwrap();
        assert_eq!((t.year, t.month, t.day, t.hour, t.minute, t.second), (2026, 10, 18, 4, 3, 2));
        // ManualSave_52586_7ea50c00_5a98ce3: 2026-11-04 22:41:35.227.
        let t = SaveTime::from_bc(0x7ea50c00, 0x5a98ce3).unwrap();
        assert_eq!((t.year, t.month, t.day, t.hour, t.minute, t.second), (2026, 11, 4, 22, 41, 35));
        // ManualSave_51d43_7e046000_59c1cd2: 2016-09-25 (the file is a 2016 save).
        let t = SaveTime::from_bc(0x7e046000, 0x59c1cd2).unwrap();
        assert_eq!((t.year, t.month, t.day), (2016, 9, 25));
        // The ids we invented for the first gate saves are plausible dates too.
        assert!(SaveTime::from_bc(0x7ea4cc00, 0x10309b7).is_some());
    }

    #[test]
    fn pack_unpack_round_trip() {
        let t = at(2026, 11, 4, 22, 41, 35, 230);
        assert_eq!((t.b(), t.c()), (0x7ea50c00, 0x5a98ce6));
        assert_eq!(SaveTime::from_bc(t.b(), t.c()), Some(t));
        for t in [at(2000, 1, 1, 0, 0, 0, 0), at(2099, 12, 31, 23, 59, 59, 999), at(2028, 2, 29, 12, 0, 1, 7)] {
            assert_eq!(SaveTime::from_bc(t.b(), t.c()), Some(t));
        }
    }

    #[test]
    fn implausible_ids_do_not_decode() {
        assert_eq!(SaveTime::from_bc(2, 3), None, "ManualSave_1_2_3");
        assert_eq!(SaveTime::from_bc(0x7ea50c01, 0x5a98ce3), None, "low bits of b set");
        assert_eq!(SaveTime::from_bc(0x7ea50c00 | 12 << 15, 0), None, "month 13");
        assert_eq!(SaveTime::from_bc(2026 << 20 | 1 << 15 | 30 << 10, 0), None, "31 February");
        assert_eq!(SaveTime::from_bc(0x7ea50c00, 24 << 22), None, "hour 24");
        assert_eq!(SaveTime::from_bc(0x7ea50c00, 60 << 16), None, "minute 60");
        assert_eq!(SaveTime::from_bc(0x7ea50c00, 1000), None, "millisecond 1000");
    }

    #[test]
    fn unix_millis_round_trip_and_carry() {
        let t = at(2026, 12, 31, 23, 59, 59, 999);
        let n = t.plus_millis(1);
        assert_eq!(n, at(2027, 1, 1, 0, 0, 0, 0));
        assert!(n > t);
        assert_eq!(SaveTime::from_unix_millis(0), at(1970, 1, 1, 0, 0, 0, 0));
        assert_eq!(SaveTime::from_unix_millis(t.to_unix_millis()), t);
    }

    #[test]
    fn stems_parse_by_shape() {
        let g = parse_stem("ManualSave_52586_7ea4c400_1030937").unwrap();
        assert_eq!((g.prefix.as_str(), g.a.as_str(), g.b, g.c), ("ManualSave", "52586", 0x7ea4c400, 0x1030937));
        assert_eq!(parse_stem("CheckPoint_8559a_7ea50c00_5a89e13").unwrap().a, "8559a");
        assert_eq!(parse_stem("QuickSave_1_2_3").unwrap().c, 3);
        for bad in [
            "AnotherLife_20261104_172834",
            "ManualSave_1_2",
            "ManualSave_1_2_3_4",
            "ManualSave_G_2_3",
            "ManualSave_A_2_3", // uppercase: the game writes lowercase
            "ManualSave__2_3",
            "_1_2_3",
            "ManualSave_123456789_2_3",
            "my save",
        ] {
            assert_eq!(parse_stem(bad), None, "{bad}");
        }
    }

    #[test]
    fn next_stem_has_the_game_shape_and_keeps_a() {
        let now = at(2026, 11, 5, 9, 5, 7, 42);
        let s = next_stem(Some("ManualSave_52586_7ea50c00_5a98ce3"), now, |_| false);
        assert_eq!(s, "ManualSave_52586_7ea51000_2451c2a");
        let g = parse_stem(&s).unwrap();
        assert_eq!(g.prefix, "ManualSave");
        assert_eq!(g.a, "52586");
        assert_eq!(SaveTime::from_bc(g.b, g.c), Some(now));
        // `a` also comes from a CheckPoint/QuickSave template; the output is still a ManualSave.
        assert!(next_stem(Some("CheckPoint_8559a_7ea50c00_5a89e13"), now, |_| false).starts_with("ManualSave_8559a_"));
        // Anything else gets the documented default.
        for odd in [Some("AnotherLife_20261104_172834"), Some("my copy"), None] {
            assert!(next_stem(odd, now, |_| false).starts_with(&format!("ManualSave_{DEFAULT_QUEST_KEY}_")), "{odd:?}");
        }
    }

    #[test]
    fn next_stem_is_larger_than_the_template() {
        let tmpl = "ManualSave_52586_7ea50c00_5a98ce3"; // 2026-11-04 22:41:35.227
        let big = |s: &str| {
            let g = parse_stem(s).unwrap();
            (g.b as u64) << 32 | g.c as u64
        };
        let t = big(tmpl);
        // later today: the stamp is just "now"
        let later = at(2026, 11, 4, 23, 0, 0, 0);
        assert!(big(&next_stem(Some(tmpl), later, |_| false)) > t);
        // the template's clock is ahead of ours (local time vs UTC): one second after the template
        let behind = at(2026, 11, 4, 18, 42, 0, 0);
        let s = next_stem(Some(tmpl), behind, |_| false);
        assert!(big(&s) > t);
        let g = parse_stem(&s).unwrap();
        assert_eq!(SaveTime::from_bc(g.b, g.c), Some(at(2026, 11, 4, 22, 41, 36, 227)));
        // exactly the template's time
        let same = at(2026, 11, 4, 22, 41, 35, 227);
        assert!(big(&next_stem(Some(tmpl), same, |_| false)) > t);
        // a template whose ids are not a date sets no floor
        let s = next_stem(Some("ManualSave_1_2_3"), later, |_| false);
        assert_eq!(SaveTime::from_bc(parse_stem(&s).unwrap().b, parse_stem(&s).unwrap().c), Some(later));
    }

    #[test]
    fn next_stem_avoids_taken_names() {
        let now = at(2026, 11, 5, 9, 5, 7, 42);
        let first = next_stem(Some("ManualSave_52586_7ea50c00_5a98ce3"), now, |_| false);
        let mut seen = vec![first.clone()];
        for _ in 0..5 {
            let s = next_stem(Some("ManualSave_52586_7ea50c00_5a98ce3"), now, |s| seen.iter().any(|x| x == s));
            assert!(!seen.contains(&s), "{s} clashes");
            seen.push(s);
        }
        // each retry is the next millisecond, so the names stay in order
        let mut sorted = seen.clone();
        sorted.sort_by_key(|s| {
            let g = parse_stem(s).unwrap();
            (g.b, g.c)
        });
        assert_eq!(sorted, seen);
        // a hostile `taken` cannot hang the call
        let _ = next_stem(None, now, |_| true);
    }
}
