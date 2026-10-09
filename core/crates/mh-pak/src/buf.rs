//! Little-endian cursor: the FArchive reads the pak and package readers need (port of `ue_pak_buf.gd` `UePakBuf`;
//! UE 4.26 Runtime/Core/Public/Serialization/Archive.h; CUE4Parse UE4/Readers/FArchive.cs).
//!
//! A cooked package is two pak entries, the `.uasset` header and the `.uexp` export data, addressed as one byte range
//! (export SerialOffset continues past TotalHeaderSize into the `.uexp`). The cursor reads over both slices without
//! concatenating them; only a read that straddles the seam copies (a few bytes, at most once per package).
//!
//! A read past the end returns 0 / "" and sets `bad` instead of failing, so a value the property reader mis-decodes is
//! caught by its caller (`tagged`) and skipped by the tag's size (same contract as UePakBuf).

use std::borrow::Cow;

#[derive(Clone)]
pub struct Cursor<'a> {
    head: &'a [u8],
    tail: &'a [u8],
    /// Position; may run past the end (then `bad` is set), never wraps.
    pub p: i64,
    pub bad: bool,
}

impl<'a> Cursor<'a> {
    pub fn new(b: &'a [u8]) -> Self {
        Cursor { head: b, tail: &[], p: 0, bad: false }
    }
    /// A cursor over `head` followed by `tail` (the .uasset + .uexp of a split cook)
    pub fn split(head: &'a [u8], tail: &'a [u8]) -> Self {
        Cursor { head, tail, p: 0, bad: false }
    }
    pub fn len(&self) -> i64 {
        (self.head.len() + self.tail.len()) as i64
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn left(&self) -> i64 {
        self.len() - self.p
    }

    /// `n` bytes at `at` without moving; None when out of range
    pub fn at(&self, at: i64, n: usize) -> Option<Cow<'a, [u8]>> {
        if at < 0 || at + n as i64 > self.len() {
            return None;
        }
        let (a, h) = (at as usize, self.head.len());
        if a + n <= h {
            Some(Cow::Borrowed(&self.head[a..a + n]))
        } else if a >= h {
            Some(Cow::Borrowed(&self.tail[a - h..a - h + n]))
        } else {
            let mut v = Vec::with_capacity(n);
            v.extend_from_slice(&self.head[a..]);
            v.extend_from_slice(&self.tail[..n - (h - a)]);
            Some(Cow::Owned(v))
        }
    }

    // UePakBuf._has: advance by n; out of range sets `bad` and still advances
    fn take(&mut self, n: usize) -> Option<Cow<'a, [u8]>> {
        let r = self.at(self.p, n);
        if r.is_none() {
            self.bad = true;
        }
        self.p += n as i64;
        r
    }
    fn arr<const N: usize>(&mut self) -> Option<[u8; N]> {
        self.take(N).map(|c| {
            let mut a = [0u8; N];
            a.copy_from_slice(&c);
            a
        })
    }

    pub fn skip(&mut self, n: i64) {
        self.p += n;
    }
    pub fn u8(&mut self) -> u8 {
        self.arr::<1>().map_or(0, |a| a[0])
    }
    pub fn s8(&mut self) -> i8 {
        self.u8() as i8
    }
    pub fn u16(&mut self) -> u16 {
        self.arr().map_or(0, u16::from_le_bytes)
    }
    pub fn s16(&mut self) -> i16 {
        self.arr().map_or(0, i16::from_le_bytes)
    }
    pub fn u32(&mut self) -> u32 {
        self.arr().map_or(0, u32::from_le_bytes)
    }
    pub fn s32(&mut self) -> i32 {
        self.arr().map_or(0, i32::from_le_bytes)
    }
    pub fn u64(&mut self) -> u64 {
        self.arr().map_or(0, u64::from_le_bytes)
    }
    pub fn s64(&mut self) -> i64 {
        self.arr().map_or(0, i64::from_le_bytes)
    }
    /// float32 as stored (bulk vertex data, math): UePakBuf.f32raw
    pub fn f32raw(&mut self) -> f32 {
        self.arr().map_or(0.0, f32::from_le_bytes)
    }
    /// float32 handed to readers as the shortest decimal that reads back as the same float32 (UePakBuf.f32 / short32)
    pub fn f32(&mut self) -> f64 {
        short32(self.f32raw())
    }
    pub fn f64(&mut self) -> f64 {
        self.arr().map_or(0.0, f64::from_le_bytes)
    }
    pub fn bytes(&mut self, n: usize) -> Cow<'a, [u8]> {
        self.take(n).unwrap_or(Cow::Borrowed(&[]))
    }
    /// int32 at the cursor without moving (FAssetArchive.TestReadFName peeks the next FName)
    pub fn peek_s32(&self, off: i64) -> Option<i32> {
        self.at(self.p + off, 4).map(|c| i32::from_le_bytes([c[0], c[1], c[2], c[3]]))
    }

    /// FString: int32 length including the terminator; > 0 = Latin-1 bytes, < 0 = UTF-16LE code units
    /// (FArchive.cs ReadFString; UE FString operator<< in Containers/String.cpp)
    pub fn fstring(&mut self) -> String {
        let n = self.s32();
        if n == 0 {
            return String::new();
        }
        if n > 0 {
            let Some(raw) = self.take(n as usize) else { return String::new() };
            let raw = &raw[..raw.len() - 1];
            // Latin-1: every byte is the code point (ASCII is the common case)
            return raw.iter().map(|&c| c as char).collect();
        }
        if n < -(1 << 24) {
            self.bad = true;
            return String::new();
        }
        let units = (-n) as usize;
        let Some(raw) = self.take(units * 2) else { return String::new() };
        let u: Vec<u16> = raw[..raw.len() - 2].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        String::from_utf16_lossy(&u)
    }
}

/// A float32 as the shortest decimal that reads back as the same float32 (1-9 significant digits), as a double:
/// 0.56f is 0.5600000023841858 widened, but CUE4Parse writes "0.56" to extract/json (Newtonsoft's shortest round-trip
/// form), so both backends hand the readers the same double. `short32(x) as f32 == x` always holds.
/// Port of UePakBuf.short32 (same search: increasing significant digits, ties to the even digit).
pub fn short32(x: f32) -> f64 {
    let d = x as f64;
    if d == 0.0 || !d.is_finite() {
        return d;
    }
    // exact in binary64 up to 1e22, so round(x * 10^k) / 10^k is correctly rounded
    const POW10: [f64; 23] = [
        1.0, 1e1, 1e2, 1e3, 1e4, 1e5, 1e6, 1e7, 1e8, 1e9, 1e10, 1e11, 1e12, 1e13, 1e14, 1e15, 1e16, 1e17, 1e18, 1e19,
        1e20, 1e21, 1e22,
    ];
    let e = (d.abs().ln() / 10f64.ln()).floor() as i32;
    for sig in 1..10 {
        let k = sig - 1 - e; // decimal places that keep `sig` significant digits
        let c = if (0..=22).contains(&k) {
            round_even(d * POW10[k as usize]) / POW10[k as usize]
        } else if k < 0 && -k <= 22 {
            round_even(d / POW10[(-k) as usize]) * POW10[(-k) as usize]
        } else {
            return d;
        };
        if c as f32 == x {
            return c;
        }
    }
    d
}

// Round half to even: when both neighbours of an exact tie read back as the same float32 (2689.40625 -> 2689.4062 or
// 2689.4063), .NET's shortest form keeps the even last digit (extract/json has 2689.4062)
fn round_even(x: f64) -> f64 {
    if x - x.floor() == 0.5 {
        2.0 * (x / 2.0).round()
    } else {
        x.round()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn short32_matches_cue4parse_json() {
        assert_eq!(short32(0.56), 0.56);
        assert_eq!(short32(2689.40625), 2689.4062);
        assert_eq!(short32(1.0), 1.0);
        assert_eq!(short32(-100.5), -100.5);
        for &x in &[1e-30f32, 3.4e38, 1.17549435e-38, 0.1, 123456.79, 7.0e-45] {
            assert_eq!(short32(x) as f32, x);
        }
    }
    #[test]
    fn cursor_reads_across_the_seam() {
        let (h, t) = ([1u8, 2], [3u8, 4, 5]);
        let mut c = Cursor::split(&h, &t);
        c.p = 1;
        assert_eq!(c.u16(), 0x0302);
        assert_eq!(c.u16(), 0x0504);
        assert!(!c.bad);
        assert_eq!(c.u8(), 0);
        assert!(c.bad);
    }
    #[test]
    fn fstring_latin1_and_utf16() {
        let b = [3u8, 0, 0, 0, b'h', 0xe9, 0, 0xfd, 0xff, 0xff, 0xff, b'o', 0, b'k', 0, 0, 0];
        let mut c = Cursor::new(&b);
        assert_eq!(c.fstring(), "h\u{e9}");
        assert_eq!(c.fstring(), "ok");
        assert!(!c.bad);
    }
}
