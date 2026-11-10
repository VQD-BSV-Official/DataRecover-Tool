use super::{bits_clear, u16le, u32le, u64le, Entry, FileSystem, Part, ReadAt, Run};
use std::{collections::HashSet, io};

pub struct ExFat {
    fat: Vec<u8>,
    bm: Vec<u8>, // allocation bitmap, bit i = cluster i+2
    csz: u64,
    heap: u64, // offset tuyệt đối của cluster 2
    n: u32,
    root: u32,
}

pub fn open(r: &dyn ReadAt, p: Part) -> io::Result<Option<ExFat>> {
    let mut b = [0u8; 512];
    r.read_at(p.start, &mut b)?;
    if &b[3..11] != b"EXFAT   " {
        return Ok(None);
    }
    let (bs, sh) = (b[108], b[109]);
    if !(9..=12).contains(&bs) || sh > 25 - bs {
        return Ok(None);
    }
    let (fo, fl, ho) = (u32le(&b, 80) as u64, u32le(&b, 84) as u64, u32le(&b, 88) as u64);
    let (n, root) = (u32le(&b, 92), u32le(&b, 96));
    let flen = fl << bs;
    if flen > 1 << 30 || flen < (n as u64 + 2) * 4 {
        return Ok(None);
    }
    let mut fat = vec![0; flen as usize];
    r.read_at(p.start + (fo << bs), &mut fat)?;
    let mut x = ExFat { fat, bm: vec![], csz: 1 << (bs + sh), heap: p.start + (ho << bs), n, root };
    // Bitmap nằm trong entry 0x81 của thư mục gốc
    let cs = x.chain(root, (4 << 20) / x.csz as usize + 1);
    if let Ok(d) = x.load(r, &cs) {
        if let Some(e) = d.chunks_exact(32).find(|e| e[0] == 0x81) {
            let bc = x.span(u32le(e, 20), u64le(e, 24), false);
            x.bm = x.load(r, &bc).unwrap_or_default();
        }
    }
    Ok(Some(x))
}

impl ExFat {
    fn off(&self, c: u32) -> u64 {
        self.heap + (c as u64 - 2) * self.csz
    }

    fn chain(&self, mut c: u32, max: usize) -> Vec<u32> {
        let mut v = vec![];
        while (2..self.n + 2).contains(&c) && v.len() < max {
            v.push(c);
            c = u32le(&self.fat, c as usize * 4);
        }
        v
    }

    /// Cluster chứa `size` byte từ `c`: theo FAT nếu chuỗi còn nguyên, ngược lại đoán liên tục.
    fn span(&self, c: u32, size: u64, nofat: bool) -> Vec<u32> {
        let k = size.div_ceil(self.csz);
        if c < 2 || c as u64 + k > self.n as u64 + 2 {
            return vec![];
        }
        if !nofat {
            let v = self.chain(c, k as usize);
            if v.len() as u64 == k {
                return v;
            }
        }
        (c..c + k as u32).collect()
    }

    fn load(&self, r: &dyn ReadAt, cs: &[u32]) -> io::Result<Vec<u8>> {
        if cs.len() as u64 * self.csz > 1 << 29 {
            return Err(io::ErrorKind::InvalidData.into());
        }
        let mut b = vec![0; cs.len() * self.csz as usize];
        for (&c, o) in cs.iter().zip(b.chunks_mut(self.csz as usize)) {
            r.read_at(self.off(c), o)?;
        }
        Ok(b)
    }

    fn free(&self, cs: &[u32]) -> bool {
        !cs.is_empty() && cs.iter().all(|&c| bits_clear(&self.bm, c as u64 - 2, 1))
    }

    fn runs(&self, cs: &[u32], size: u64) -> Vec<Run> {
        let mut v: Vec<Run> = vec![];
        for &c in cs {
            let o = self.off(c);
            match v.last_mut() {
                Some(l) if l.off + l.len == o => l.len += self.csz,
                _ => v.push(Run { off: o, len: self.csz }),
            }
        }
        if let Some(l) = v.last_mut() {
            l.len -= cs.len() as u64 * self.csz - size; // bỏ phần dư của cluster cuối
        }
        v
    }
}

impl FileSystem for ExFat {
    fn name(&self) -> &'static str {
        "exFAT"
    }

    fn deleted(&self, r: &dyn ReadAt, emit: &mut dyn FnMut(Entry)) -> io::Result<()> {
        let mut seen = HashSet::new();
        let mut st = vec![(String::new(), self.chain(self.root, (4 << 20) / self.csz as usize + 1), false)];
        while let Some((path, cs, del)) = st.pop() {
            let Ok(d) = self.load(r, &cs) else { continue };
            let mut it = d.chunks_exact(32);
            while let Some(e) = it.next() {
                match e[0] {
                    0 => break,
                    0x85 | 0x05 => {} // file/dir: đang dùng | đã xóa (bit 7 bị xóa)
                    _ => continue,
                }
                let sec: Vec<&[u8]> = it.by_ref().take(e[1] as usize).collect();
                let Some(s) = sec.iter().find(|s| s[0] & 0x7F == 0x40) else { continue };
                let w: Vec<u16> = sec
                    .iter()
                    .filter(|s| s[0] & 0x7F == 0x41)
                    .flat_map(|s| (1..16).map(move |i| u16le(s, i * 2)))
                    .take(s[3] as usize)
                    .collect();
                let (vdl, c, dl) = (u64le(s, 8), u32le(s, 20), u64le(s, 24));
                let (isdel, nofat) = (del || e[0] == 0x05, s[1] & 2 != 0);
                let p = format!("{path}/{}", String::from_utf16_lossy(&w));
                if u16le(e, 4) & 0x10 != 0 {
                    if dl > 1 << 26 {
                        continue;
                    }
                    let cs = self.span(c, dl, nofat);
                    if !cs.is_empty() && seen.insert(c) && (!isdel || self.free(&cs)) {
                        st.push((p, cs, isdel));
                    }
                } else if isdel && vdl > 0 {
                    let size = vdl.min(dl);
                    let cs = self.span(c, size, nofat);
                    if !cs.is_empty() {
                        emit(Entry { path: p, size, runs: self.runs(&cs, size), ok: self.free(&cs) });
                    }
                }
            }
        }
        Ok(())
    }
}
