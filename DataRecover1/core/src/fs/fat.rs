use super::{u16le, u32le, Entry, FileSystem, Part, ReadAt, Run};
use std::{collections::HashSet, io};

/// FAT16 / FAT32 (không hỗ trợ FAT12).
pub struct Fat {
    fat: Vec<u8>,
    csz: u64,
    data: u64,        // offset tuyệt đối của cluster 2
    root: (u64, u64), // vùng root cố định của FAT16: (offset, độ dài)
    root_cl: u32,     // 0 = FAT16
    n: u32,           // số cluster dữ liệu
    fat32: bool,
}

pub fn open(r: &dyn ReadAt, p: Part) -> io::Result<Option<Fat>> {
    let mut b = [0u8; 512];
    r.read_at(p.start, &mut b)?;
    let (bps, spc, rsv, nf, re) = (u16le(&b, 11) as u64, b[13] as u64, u16le(&b, 14) as u64, b[16] as u64, u16le(&b, 17) as u64);
    if b[510..] != [0x55, 0xAA] || !matches!(bps, 512 | 1024 | 2048 | 4096) || !spc.is_power_of_two() || spc > 128 || rsv == 0 || !(1..=4).contains(&nf) {
        return Ok(None);
    }
    let fsz = match u16le(&b, 22) { 0 => u32le(&b, 36), x => x as u32 } as u64;
    let total = match u16le(&b, 19) { 0 => u32le(&b, 32), x => x as u32 } as u64;
    let root_secs = (re * 32).div_ceil(bps);
    let first = rsv + nf * fsz + root_secs; // sector đầu vùng dữ liệu
    if fsz == 0 || total <= first {
        return Ok(None);
    }
    let n = ((total - first) / spc).min(0x0FFF_FFF5) as u32;
    let fat32 = n >= 65525;
    let (flen, es) = (fsz * bps, if fat32 { 4 } else { 2 });
    if n < 4085 || flen > 1 << 30 || flen < (n as u64 + 2) * es {
        return Ok(None);
    }
    let mut fat = vec![0; flen as usize];
    r.read_at(p.start + rsv * bps, &mut fat)?;
    Ok(Some(Fat {
        fat,
        csz: bps * spc,
        data: p.start + first * bps,
        root: (p.start + (rsv + nf * fsz) * bps, root_secs * bps),
        root_cl: if fat32 { u32le(&b, 44) } else { 0 },
        n,
        fat32,
    }))
}

impl Fat {
    fn valid(&self, c: u32) -> bool {
        (2..self.n + 2).contains(&c)
    }

    fn off(&self, c: u32) -> u64 {
        self.data + (c as u64 - 2) * self.csz
    }

    fn ent(&self, c: u32) -> u32 {
        if self.fat32 {
            u32le(&self.fat, c as usize * 4) & 0x0FFF_FFFF
        } else {
            u16le(&self.fat, c as usize * 2) as u32
        }
    }

    /// Chuỗi cluster theo FAT, tối đa `max` (chặn vòng lặp).
    fn chain(&self, mut c: u32, max: usize) -> Vec<u32> {
        let eoc = if self.fat32 { 0x0FFF_FFF8 } else { 0xFFF8 };
        let mut v = vec![];
        while self.valid(c) && v.len() < max {
            v.push(c);
            c = self.ent(c);
            if c >= eoc {
                break;
            }
        }
        v
    }

    /// Nội dung thư mục. `del`: thư mục đã xóa, chuỗi FAT mất nên chỉ đọc cluster đầu.
    fn dir(&self, r: &dyn ReadAt, c: u32, del: bool) -> io::Result<Vec<u8>> {
        if c == 0 {
            let mut b = vec![0; self.root.1 as usize];
            r.read_at(self.root.0, &mut b)?;
            return Ok(b);
        }
        let cs = if del { vec![c] } else { self.chain(c, (2 << 20) / self.csz as usize + 1) };
        let mut b = vec![0; cs.len() * self.csz as usize];
        for (&c, o) in cs.iter().zip(b.chunks_mut(self.csz as usize)) {
            r.read_at(self.off(c), o)?;
        }
        Ok(b)
    }
}

impl FileSystem for Fat {
    fn name(&self) -> &'static str {
        if self.fat32 { "FAT32" } else { "FAT16" }
    }

    fn deleted(&self, r: &dyn ReadAt, emit: &mut dyn FnMut(Entry)) -> io::Result<()> {
        let mut seen = HashSet::new();
        let mut st = vec![(String::new(), self.root_cl, false)];
        while let Some((path, cl, del)) = st.pop() {
            let Ok(buf) = self.dir(r, cl, del) else { continue };
            let mut lfn: Vec<[u16; 13]> = vec![];
            let (mut sum, mut lfn_del) = (0, true);
            for e in buf.chunks_exact(32) {
                if e[0] == 0 {
                    break;
                }
                if e[11] & 0x3F == 0x0F {
                    if lfn.is_empty() {
                        lfn_del = true;
                    }
                    lfn_del &= e[0] == 0xE5;
                    sum = e[13];
                    let mut u = [0u16; 13];
                    for (i, o) in [1, 3, 5, 7, 9, 14, 16, 18, 20, 22, 24, 28, 30].into_iter().enumerate() {
                        u[i] = u16le(e, o);
                    }
                    lfn.push(u);
                    continue;
                }
                let a = e[11];
                let dot = e[0] == b'.' && matches!(e[1], b'.' | b' ');
                let is_del = del || e[0] == 0xE5;
                let (size, mut c) = (u32le(e, 28) as u64, u16le(e, 26) as u32);
                if self.fat32 {
                    c |= (u16le(e, 20) as u32) << 16;
                }
                let isdir = a & 0x10 != 0;
                if a & 8 == 0 && !dot && self.valid(c) && (isdir || (is_del && size > 0)) {
                    // Entry đã xóa: byte đầu tên bị ghi đè nên checksum LFN không kiểm được, tin vào cờ xóa của LFN.
                    let lfn_ok = !lfn.is_empty() && if e[0] == 0xE5 { lfn_del } else { sum == cksum(&e[..11]) };
                    let name = if lfn_ok {
                        let w: Vec<u16> = lfn.iter().rev().flatten().copied().take_while(|&x| x != 0 && x != 0xFFFF).collect();
                        String::from_utf16_lossy(&w)
                    } else {
                        short(e)
                    };
                    let p = format!("{path}/{name}");
                    if isdir {
                        if seen.insert(c) && (!is_del || self.ent(c) == 0) {
                            st.push((p, c, is_del));
                        }
                    } else {
                        let k = size.div_ceil(self.csz);
                        if c as u64 + k <= self.n as u64 + 2 {
                            emit(Entry {
                                path: p,
                                size,
                                runs: vec![Run { off: self.off(c), len: size }],
                                ok: (c..c + k as u32).all(|x| self.ent(x) == 0),
                            });
                        }
                    }
                }
                lfn.clear();
            }
        }
        Ok(())
    }
}

fn cksum(n: &[u8]) -> u8 {
    n.iter().fold(0, |s: u8, &c| s.rotate_right(1).wrapping_add(c))
}

fn short(e: &[u8]) -> String {
    let t = |s: &[u8]| s.iter().map(|&c| c as char).collect::<String>().trim_end().to_owned();
    let mut b = [0u8; 8];
    b.copy_from_slice(&e[..8]);
    match b[0] {
        0xE5 => b[0] = b'_', // ký tự đầu của entry đã xóa không còn
        0x05 => b[0] = 0xE5,
        _ => {}
    }
    let (n, x) = (t(&b), t(&e[8..11]));
    if x.is_empty() { n } else { format!("{n}.{x}") }
}
