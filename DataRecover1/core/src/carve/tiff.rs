use super::{Kind, Src};

// kích thước theo type TIFF 1..=12
const SZ: [u64; 13] = [0, 1, 1, 2, 4, 8, 1, 1, 2, 4, 8, 4, 8];

struct W<'a, S> {
    s: &'a S,
    base: u64,
    be: bool,
    end: u64,
    q: Vec<u64>,
    seen: Vec<u64>,
    make: [u8; 5],
}

impl<S: Src> W<'_, S> {
    fn u16(&self, b: &[u8]) -> u64 {
        let b = [b[0], b[1]];
        (if self.be { u16::from_be_bytes(b) } else { u16::from_le_bytes(b) }) as u64
    }
    fn u32(&self, b: &[u8]) -> u64 {
        let b = [b[0], b[1], b[2], b[3]];
        (if self.be { u32::from_be_bytes(b) } else { u32::from_le_bytes(b) }) as u64
    }
    fn num(&self, t: u64, b: &[u8]) -> u64 { if t == 3 { self.u16(b) } else { self.u32(b) } }

    /// Mảng SHORT/LONG của entry (inline hoặc tại offset).
    fn vals(&self, t: u64, n: u64, v: &[u8]) -> Vec<u64> {
        if !(t == 3 || t == 4) || n > 1 << 16 { return vec![]; }
        let (sz, len) = (SZ[t as usize], (SZ[t as usize] * n) as usize);
        let d;
        let b = if len <= 4 { &v[..len] } else {
            match self.s.read_vec(self.base + self.u32(v), len) { Some(x) => { d = x; &d[..] } None => return vec![] }
        };
        b.chunks_exact(sz as usize).map(|c| self.num(t, c)).collect()
    }

    fn ifd(&mut self, o: u64, first: bool) -> Option<()> {
        let n = self.u16(&self.s.rd::<2>(self.base + o)?);
        if n == 0 || n > 512 { return None; }
        let b = self.s.read_vec(self.base + o + 2, n as usize * 12 + 4)?;
        self.end = self.end.max(o + 2 + n * 12 + 4);
        let (mut so, mut sc) = (vec![], vec![]);
        let mut r = [0u64; 4]; // JPEG preview off/len, Sony SR2 off/len
        for e in b.chunks_exact(12) {
            let (tag, t, c, v) = (self.u16(&e[..2]), self.u16(&e[2..4]), self.u32(&e[4..8]), &e[8..]);
            if t == 0 || t > 12 { return None; }
            let len = SZ[t as usize] * c;
            if len > 4 { self.end = self.end.max(self.u32(v) + len); }
            match tag {
                0x010F if first => {
                    self.make = if len > 4 { self.s.rd(self.base + self.u32(v))? } else {
                        let mut m = [0; 5];
                        m[..4].copy_from_slice(v);
                        m
                    };
                }
                0x0111 | 0x0144 => so = self.vals(t, c, v),
                0x0117 | 0x0145 => sc = self.vals(t, c, v),
                0x0201 => r[0] = self.num(t, v),
                0x0202 => r[1] = self.num(t, v),
                0x7200 => r[2] = self.num(t, v),
                0x7201 => r[3] = self.num(t, v),
                0x014A => { let x = self.vals(t, c, v); self.q.extend(x); }
                0x8769 | 0x8825 | 0xA005 => self.q.push(self.num(t, v)),
                _ => {}
            }
        }
        for (a, l) in so.iter().zip(&sc) { self.end = self.end.max(a + l); }
        self.end = self.end.max(r[0] + r[1]).max(r[2] + r[3]);
        let nx = self.u32(&b[n as usize * 12..]);
        if nx != 0 { self.q.push(nx); }
        Some(())
    }
}

pub fn probe(s: &impl Src, base: u64, be: bool) -> Option<(Kind, u64)> {
    let h: [u8; 16] = s.rd(base)?;
    let mut w = W { s, base, be, end: 16, q: vec![], seen: vec![], make: [0; 5] };
    let o = w.u32(&h[4..8]);
    if !(8..1 << 24).contains(&o) { return None; }
    w.seen.push(o);
    w.ifd(o, true)?;
    let kind = if &h[8..10] == b"CR" { Kind::Cr2 }
        else if &w.make == b"NIKON" { Kind::Nef }
        else if &w.make[..4] == b"SONY" { Kind::Arw }
        else { return None };
    while let Some(o) = w.q.pop() {
        if w.seen.len() >= 64 { break; }
        if !w.seen.contains(&o) {
            w.seen.push(o);
            let _ = w.ifd(o, false);
        }
    }
    (base + w.end <= s.len()).then_some((kind, w.end))
}
