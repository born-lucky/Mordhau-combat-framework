//! Little-endian cursor over a byte slice: the FArchive reads the asset decoders need (UE 4.26
//! Runtime/Core/Public/Serialization/Archive.h; CUE4Parse UE4/Readers/FArchive.cs). Port of
//! godot/components/ue/pak/ue_pak_buf.gd: a read past the end returns 0 / "" and sets `bad` instead of panicking, so
//! the caller checks `bad` once after a run of reads.

pub struct Buf<'a> {
    pub b: &'a [u8],
    pub p: usize,
    pub bad: bool,
}

impl<'a> Buf<'a> {
    pub fn new(b: &'a [u8]) -> Self {
        Buf { b, p: 0, bad: false }
    }

    pub fn at(b: &'a [u8], p: usize) -> Self {
        Buf { b, p, bad: false }
    }

    /// The `n` bytes at the cursor (advancing), or None (and `bad`) past the end
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        match self.p.checked_add(n) {
            Some(e) if e <= self.b.len() => {
                let s = &self.b[self.p..e];
                self.p = e;
                Some(s)
            }
            _ => {
                self.bad = true;
                self.p = self.p.saturating_add(n);
                None
            }
        }
    }

    pub fn skip(&mut self, n: usize) {
        self.p = self.p.saturating_add(n);
    }

    /// skip by a count read from the data (negative counts mark the buffer bad)
    pub fn skip_n(&mut self, count: i32, elem: usize) {
        if count < 0 {
            self.bad = true;
            return;
        }
        self.skip(count as usize * elem);
    }

    pub fn u8(&mut self) -> u8 {
        self.take(1).map_or(0, |s| s[0])
    }
    pub fn s8(&mut self) -> i8 {
        self.u8() as i8
    }
    pub fn u16(&mut self) -> u16 {
        self.take(2).map_or(0, |s| u16::from_le_bytes([s[0], s[1]]))
    }
    pub fn s16(&mut self) -> i16 {
        self.u16() as i16
    }
    pub fn u32(&mut self) -> u32 {
        self.take(4).map_or(0, |s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    }
    pub fn s32(&mut self) -> i32 {
        self.u32() as i32
    }
    pub fn u64(&mut self) -> u64 {
        self.take(8).map_or(0, |s| u64::from_le_bytes(s.try_into().unwrap()))
    }
    pub fn s64(&mut self) -> i64 {
        self.u64() as i64
    }
    pub fn f32(&mut self) -> f32 {
        f32::from_bits(self.u32())
    }
    pub fn f64(&mut self) -> f64 {
        f64::from_bits(self.u64())
    }
    /// IEEE half (FFloat16), widened exactly
    pub fn f16(&mut self) -> f32 {
        half_to_f32(self.u16())
    }
    pub fn bytes(&mut self, n: usize) -> &'a [u8] {
        self.take(n).unwrap_or(&[])
    }
    pub fn left(&self) -> usize {
        self.b.len().saturating_sub(self.p)
    }

    /// FString: int32 length including the terminator; > 0 = Latin-1 bytes, < 0 = UTF-16LE code units
    /// (FArchive.cs ReadFString; UE FString operator<< in Containers/String.cpp)
    pub fn fstring(&mut self) -> String {
        let n = self.s32();
        if n == 0 {
            return String::new();
        }
        if n > 0 {
            let raw = self.bytes(n as usize);
            if raw.is_empty() {
                return String::new();
            }
            return raw[..raw.len() - 1].iter().map(|&c| c as char).collect();
        }
        if n < -(1 << 24) {
            self.bad = true;
            return String::new();
        }
        let raw = self.bytes((-n) as usize * 2);
        if raw.len() < 2 {
            return String::new();
        }
        let units: Vec<u16> = raw[..raw.len() - 2].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        String::from_utf16_lossy(&units)
    }

    /// int32 element size + int32 count + the elements (CUE4Parse ReadBulkArray); `esz` checked when Some
    pub fn bulk_array(&mut self, esz: Option<usize>) -> (usize, &'a [u8]) {
        let e = self.s32();
        let n = self.s32();
        if e < 0 || n < 0 || esz.is_some_and(|w| n > 0 && w != e as usize) {
            self.bad = true;
            return (0, &[]);
        }
        let s = self.bytes(e as usize * n as usize);
        (n as usize, s)
    }
}

/// IEEE 754 binary16 -> f32 (exact; subnormals, inf and NaN kept)
pub fn half_to_f32(h: u16) -> f32 {
    let s = ((h >> 15) as u32) << 31;
    let e = ((h >> 10) & 0x1f) as u32;
    let m = (h & 0x3ff) as u32;
    let bits = if e == 0 {
        if m == 0 {
            s
        } else {
            // subnormal: m * 2^-24
            let v = m as f32 * (1.0 / 16_777_216.0);
            return if s != 0 { -v } else { v };
        }
    } else if e == 31 {
        s | 0x7f80_0000 | (m << 13)
    } else {
        s | ((e + 112) << 23) | (m << 13)
    };
    f32::from_bits(bits)
}

pub fn le_u16(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}
pub fn le_u32(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}
pub fn le_i32(b: &[u8], o: usize) -> i32 {
    le_u32(b, o) as i32
}
pub fn le_f32(b: &[u8], o: usize) -> f32 {
    f32::from_bits(le_u32(b, o))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn half_matches_known_values() {
        assert_eq!(half_to_f32(0x3c00), 1.0);
        assert_eq!(half_to_f32(0xc000), -2.0);
        assert_eq!(half_to_f32(0x3555), 0.333_251_95);
        assert_eq!(half_to_f32(0x0001), 5.960_464_5e-8);
        assert_eq!(half_to_f32(0x7bff), 65504.0);
        assert!(half_to_f32(0x7c00).is_infinite());
    }

    #[test]
    fn reads_past_end_set_bad() {
        let d = [1u8, 0, 0, 0];
        let mut r = Buf::new(&d);
        assert_eq!(r.s32(), 1);
        assert!(!r.bad);
        assert_eq!(r.u16(), 0);
        assert!(r.bad);
    }

    #[test]
    fn fstring_latin1_and_utf16() {
        let d = [3u8, 0, 0, 0, b'h', 0xe9, 0, 0xfd, 0xff, 0xff, 0xff, b'o', 0, b'k', 0, 0, 0];
        let mut r = Buf::new(&d);
        assert_eq!(r.fstring(), "h\u{e9}");
        assert_eq!(r.fstring(), "ok");
        assert!(!r.bad);
    }
}
