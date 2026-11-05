//! `.w3strings`: the localized string files of the player's own install
//! (`content\content0\<lang>.w3strings`, `dlc\*\content\<lang>.w3strings`).
//!
//! Format, worked out from the 5.01 files themselves (no third-party code; nothing from the files is
//! stored in the repository, everything is read at run time):
//!
//! ```text
//! "RTSW"            4 bytes
//! u32               164 in every file seen
//! u16 key           4 BCD-looking digits (0x4397 in en); NOT needed to decode (see below)
//! varint count      number of strings (A)
//! A: count x { u32 id, u32 offset, u32 len }      sorted by id, 12 bytes each
//! varint count      number of named keys (B)
//! u32               95 in every file seen
//! B: count x { u32 id, u32 key_hash }             sorted by hash, 8 bytes each; id is an A id
//! data              strings: A.offset is relative to here; every string is len bytes + NUL
//! ```
//!
//! The varint is `ReadBit6`: first byte = 6 value bits, bit 6 = more follows (bit 7 = sign, never set
//! here); later bytes = 7 value bits, bit 7 = more follows; little-endian groups.
//!
//! **Text** is UTF-8, `len` is the byte length (without the NUL), obfuscated with a per-file 16-bit
//! constant `W` (0 = stored plain): `plain[i] = stored[i] ^ (((len + 1) * m_i) & 0xff)`, where `m_i`
//! is the low byte of `W` rotated left by `i mod 16` as a 16-bit value. Seen: `W` is four decimal digits
//! read as hex (en 0x3217, ru 0x3863, ...); no relation to the header key was found, so the reader finds
//! `W` by trying all 65536 values on a few strings (a wrong `W` almost never gives valid UTF-8 without
//! control characters on 40 strings). Files without a key (ar, br, cn, esmx, kr, tr, ua) are plain.
//!
//! **Ids.** The ids in A are the game's string ids XOR a per-file constant `K`
//! (`stored = id ^ K`); the same id has the same text in every language, `K` differs per file. `K` is
//! `T << 24 | W << 8 | L`: `T` is the top byte every stored id of the file shares, `W` the text constant
//! above and `L` a byte that is not derivable; [`Strings::calibrate`] finds it from ids the game itself
//! references (journal titles): the right `L` makes nearly all of them present, a wrong one ~5 %.
//!
//! **Keys.** B maps the textual keys of the XML definitions (`item_name_...`) to A ids through a 32-bit
//! hash of the key. The hash is language independent and is none of FNV-1/1a, CRC-32/32C, djb2, sdbm,
//! Jenkins OAT, Murmur2, xxHash32, x31/x37/x131/x65599 (tried on the 4587 item keys of the install, 0 hits),
//! so [`Strings::keyed`] is not usable yet and item names cannot be looked up by key.
use std::path::Path;

use crate::quest::Error;

/// One loaded `.w3strings` file.
pub struct Strings {
    data: Vec<u8>,
    /// `(stored id, offset, len)`, sorted by stored id.
    entries: Vec<(u32, u32, u32)>,
    data_start: usize,
    /// `(stored id, key hash)` of the named keys, sorted by hash.
    keys: Vec<(u32, u32)>,
    w: u16,
    /// `K` without its low byte until [`Strings::calibrate`] ran.
    k: u32,
    calibrated: bool,
}

fn varint(b: &[u8], o: &mut usize) -> Option<u32> {
    let b0 = *b.get(*o)?;
    *o += 1;
    let mut v = u32::from(b0 & 0x3f);
    let mut shift = 6;
    let mut more = b0 & 0x40 != 0;
    while more {
        let x = *b.get(*o)?;
        *o += 1;
        v |= u32::from(x & 0x7f).checked_shl(shift)?;
        shift += 7;
        more = x & 0x80 != 0;
    }
    Some(v)
}

fn u32_at(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?))
}

/// The keystream byte for position `i` of a string of `len` bytes under constant `w`.
fn ks(w: u16, len: u32, i: usize) -> u8 {
    let m = u32::from(w.rotate_left((i % 16) as u32) & 0xff);
    (len.wrapping_add(1).wrapping_mul(m) & 0xff) as u8
}

fn decode(raw: &[u8], w: u16) -> Vec<u8> {
    raw.iter().enumerate().map(|(i, &c)| c ^ ks(w, raw.len() as u32, i)).collect()
}

fn plausible(s: &[u8]) -> bool {
    match std::str::from_utf8(s) {
        Ok(t) => !t.chars().any(|c| (c < ' ' && c != '\n' && c != '\r' && c != '\t') || c == '\u{fffd}'),
        Err(_) => false,
    }
}

impl Strings {
    pub fn open(path: &Path) -> Result<Strings, Error> {
        let data = std::fs::read(path).map_err(|e| Error::Io(path.display().to_string(), e.to_string()))?;
        Strings::from_bytes(data).map_err(|e| Error::Format(format!("{}: {e}", path.display())))
    }

    pub fn from_bytes(data: Vec<u8>) -> Result<Strings, String> {
        let bad = || "not a w3strings file (RTSW)".to_owned();
        if data.get(0..4) != Some(b"RTSW") {
            return Err(bad());
        }
        let mut o = 10;
        let n = varint(&data, &mut o).ok_or_else(bad)? as usize;
        let table_end = o.checked_add(n.checked_mul(12).ok_or_else(bad)?).filter(|&e| e <= data.len()).ok_or_else(bad)?;
        let entries: Vec<(u32, u32, u32)> =
            (0..n).map(|i| (u32_at(&data, o + i * 12), u32_at(&data, o + i * 12 + 4), u32_at(&data, o + i * 12 + 8))).map(|(a, b, c)| (a.unwrap_or(0), b.unwrap_or(0), c.unwrap_or(0))).collect();
        let mut o2 = table_end;
        let m = varint(&data, &mut o2).ok_or_else(bad)? as usize;
        let kb = o2 + 4;
        let data_start = m.checked_mul(8).and_then(|x| kb.checked_add(x)).filter(|&e| e <= data.len()).ok_or_else(bad)?;
        let keys = (0..m).map(|i| (u32_at(&data, kb + i * 8).unwrap_or(0), u32_at(&data, kb + i * 8 + 4).unwrap_or(0))).collect();
        if entries.windows(2).any(|w| w[0].0 > w[1].0) {
            return Err("string table is not sorted by id".to_owned());
        }
        for e in &entries {
            if data_start + e.1 as usize + e.2 as usize >= data.len() {
                return Err("a string points outside the file".to_owned());
            }
        }
        let mut s = Strings { data, entries, data_start, keys, w: 0, k: 0, calibrated: false };
        s.w = s.find_w()?;
        let top = s.entries.first().map_or(0, |e| e.0 >> 24);
        s.k = top << 24 | u32::from(s.w) << 8;
        Ok(s)
    }

    fn raw(&self, e: &(u32, u32, u32)) -> &[u8] {
        let a = self.data_start + e.1 as usize;
        &self.data[a..a + e.2 as usize]
    }

    fn find_w(&self) -> Result<u16, String> {
        let long: Vec<&(u32, u32, u32)> = self.entries.iter().filter(|e| e.2 >= 16 && e.2 < 400).collect();
        if long.len() < 40 {
            return Err("too few strings to find the text constant".to_owned());
        }
        let step = long.len() / 40;
        let sample: Vec<&[u8]> = (0..40).map(|i| self.raw(long[i * step])).collect();
        let ok = |w: u16| sample.iter().all(|r| plausible(&decode(r, w)));
        (0..=u16::MAX).find(|&w| ok(w)).ok_or_else(|| "no text constant decodes the sample strings".to_owned())
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The text constant `W` found for this file (0 = plain).
    pub fn text_constant(&self) -> u16 {
        self.w
    }

    /// Number of named keys (B).
    pub fn key_count(&self) -> usize {
        self.keys.len()
    }

    /// Text of a stored id (no id mapping).
    pub fn get_stored(&self, stored: u32) -> Option<String> {
        let i = self.entries.binary_search_by_key(&stored, |e| e.0).ok()?;
        Some(String::from_utf8_lossy(&decode(self.raw(&self.entries[i]), self.w)).into_owned())
    }

    /// Choose the id constant's low byte `L` so that as many of `ids` as possible exist. Returns the
    /// number of hits; an error when fewer than a third of the ids (and fewer than 8) are present.
    pub fn calibrate(&mut self, ids: &[u32]) -> Result<usize, String> {
        let mut best = (0u32, 0usize);
        for l in 0..=255u32 {
            let k = (self.k & !0xff) | l;
            let hits = ids.iter().filter(|&&id| self.entries.binary_search_by_key(&(id ^ k), |e| e.0).is_ok()).count();
            if hits > best.1 {
                best = (l, hits);
            }
        }
        if best.1 < 8 || best.1 * 3 < ids.len() {
            return Err(format!("only {} of {} referenced ids found: cannot calibrate the id constant", best.1, ids.len()));
        }
        self.k = (self.k & !0xff) | best.0;
        self.calibrated = true;
        Ok(best.1)
    }

    pub fn is_calibrated(&self) -> bool {
        self.calibrated
    }

    /// Text of a game string id; `None` before [`Strings::calibrate`] or when the id is absent.
    pub fn get(&self, id: u32) -> Option<String> {
        if !self.calibrated {
            return None;
        }
        self.get_stored(id ^ self.k)
    }

    /// All `(game id, text)` pairs (after calibration).
    pub fn iter(&self) -> impl Iterator<Item = (u32, String)> + '_ {
        let k = self.k;
        let on = self.calibrated;
        self.entries.iter().filter(move |_| on).map(move |e| (e.0 ^ k, String::from_utf8_lossy(&decode(self.raw(e), self.w)).into_owned()))
    }

    /// The `(game id, text)` pairs that have a textual key (B): item names and descriptions, UI texts.
    /// After calibration only.
    pub fn named(&self) -> Vec<(u32, String)> {
        if !self.calibrated {
            return Vec::new();
        }
        self.keys.iter().filter_map(|k| self.get_stored(k.0).map(|t| (k.0 ^ self.k, t))).collect()
    }

    /// Text for a textual key by its hash. See the module docs: the hash function is unknown, so this
    /// is only usable with a precomputed hash.
    pub fn keyed(&self, hash: u32) -> Option<String> {
        let i = self.keys.binary_search_by_key(&hash, |k| k.1).ok()?;
        self.get_stored(self.keys[i].0)
    }
}

/// `<install>\content\content0\<lang>.w3strings`.
pub fn path_of(install: &Path, lang: &str) -> std::path::PathBuf {
    install.join("content").join("content0").join(format!("{lang}.w3strings"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn varint_bit6() {
        let mut o = 0;
        assert_eq!(varint(&[0x53, 0xea, 0x0b], &mut o), Some(96915));
        o = 0;
        assert_eq!(varint(&[0x40, 0x8c, 0x02], &mut o), Some(17152));
        o = 0;
        assert_eq!(varint(&[0x05], &mut o), Some(5));
    }

    #[test]
    fn keystream_roundtrip() {
        let plain = b"Hello there, Geralt";
        let w = 0x3217u16;
        let enc: Vec<u8> = plain.iter().enumerate().map(|(i, &c)| c ^ ks(w, plain.len() as u32, i)).collect();
        assert_ne!(&enc[..], &plain[..]);
        assert_eq!(decode(&enc, w), plain);
        // len + 1 = 256: the keystream is zero, the text is stored plain
        let long = vec![b'a'; 255];
        assert_eq!(decode(&long, w), long);
    }

    #[test]
    fn rejects_other_files() {
        assert!(Strings::from_bytes(b"POTATO70".to_vec()).is_err());
    }

    /// Against the real install (`W3AL_INSTALL`, else the Steam default); silently skipped without one.
    #[test]
    fn reads_the_installed_english_and_russian_files() {
        let install = std::env::var("W3AL_INSTALL").unwrap_or_else(|_| r"C:\Program Files (x86)\Steam\steamapps\common\The Witcher 3".to_owned());
        let install = Path::new(&install);
        let (Ok(mut en), Ok(mut ru)) = (Strings::open(&path_of(install, "en")), Strings::open(&path_of(install, "ru"))) else { return };
        assert!(en.len() > 50_000 && ru.len() > 50_000);
        assert_ne!(en.text_constant(), 0);
        let bundles = crate::bundle::bundles_of_install(install);
        if bundles.is_empty() {
            return;
        }
        let ids = crate::names::journal_ids(install).expect("journals");
        assert!(ids.len() > 1000);
        let hits_en = en.calibrate(&ids).expect("en calibrates");
        let hits_ru = ru.calibrate(&ids).expect("ru calibrates");
        assert!(hits_en * 10 > ids.len() * 8, "en hits {hits_en} of {}", ids.len());
        assert!(hits_ru * 10 > ids.len() * 8, "ru hits {hits_ru} of {}", ids.len());
        // the same id has a text in both languages and the two differ in script
        let sample = ids.iter().copied().find(|&i| en.get(i).is_some_and(|t| t.len() > 8) && ru.get(i).is_some()).expect("an id in both");
        let (a, b) = (en.get(sample).unwrap(), ru.get(sample).unwrap());
        assert!(a.is_ascii() || !b.is_ascii(), "en {a:?} ru {b:?}");
    }
}
