use super::{bits_clear, u16le, u32le, u64le, Entry, FileSystem, Part, ReadAt, Run, HOLE};
use std::{collections::HashMap, io};

const CH: usize = 1 << 20;
const MASK: u64 = 0xFFFF_FFFF_FFFF; // 48 bit thấp của tham chiếu MFT

pub struct Ntfs {
    base: u64,
    csz: u64,
    rs: usize,           // kích thước 1 MFT record
    mft: Vec<(u64, u64)>, // data runs của $MFT: (lcn, số cluster)
    size: u64,           // kích thước $MFT
    bm: Vec<u8>,         // $Bitmap, bit i = cluster i
}

type Runs = Vec<(Option<u64>, u64)>; // lcn None = sparse

struct Rec {
    used: bool,
    dir: bool,
    parent: u64,
    name: String,
    size: u64,
    runs: Runs,
}

pub fn open(r: &dyn ReadAt, p: Part) -> io::Result<Option<Ntfs>> {
    let mut b = [0u8; 512];
    r.read_at(p.start, &mut b)?;
    if &b[3..11] != b"NTFS    " {
        return Ok(None);
    }
    let (bps, cpr) = (u16le(&b, 11) as u64, b[64] as i8);
    let spc = match b[13] {
        s @ 1..=0x80 => s as u64,
        s @ 0xE0..=0xFF => 1 << (256 - s as u32),
        _ => return Ok(None),
    };
    let csz = bps * spc;
    let rs = if cpr > 0 { cpr as u64 * csz } else if cpr >= -16 { 1u64 << cpr.unsigned_abs() } else { 0 };
    if !(512..=4096).contains(&bps) || !(512..=65536).contains(&rs) || rs % 512 != 0 {
        return Ok(None);
    }
    let mft_off = p.start.saturating_add(u64le(&b, 48).saturating_mul(csz));
    let mut m = vec![0u8; rs as usize];
    r.read_at(mft_off, &mut m)?;
    let Some(z) = parse(&mut m) else { return Ok(None) };
    let mft: Vec<_> = z.runs.iter().filter_map(|&(l, n)| Some((l?, n))).collect();
    if mft.is_empty() {
        return Ok(None);
    }
    let mut x = Ntfs { base: p.start, csz, rs: rs as usize, mft, size: z.size, bm: vec![] };
    // Record 6 = $Bitmap, nằm ngay đầu MFT
    if r.read_at(mft_off + 6 * rs, &mut m).is_ok() {
        if let Some(z) = parse(&mut m) {
            x.bm = x.read_runs(r, &z.runs, z.size).unwrap_or_default();
        }
    }
    Ok(Some(x))
}

impl Ntfs {
    fn off(&self, lcn: u64) -> u64 {
        self.base.saturating_add(lcn.saturating_mul(self.csz))
    }

    fn read_runs(&self, r: &dyn ReadAt, runs: &Runs, size: u64) -> io::Result<Vec<u8>> {
        if size > 1 << 29 {
            return Err(io::ErrorKind::InvalidData.into());
        }
        let mut b = vec![0u8; size as usize];
        let mut pos = 0;
        for &(l, n) in runs {
            if pos == b.len() {
                break;
            }
            let len = n.saturating_mul(self.csz).min((b.len() - pos) as u64) as usize;
            if let Some(l) = l {
                r.read_at(self.off(l), &mut b[pos..pos + len])?;
            }
            pos += len;
        }
        Ok(b)
    }
}

impl FileSystem for Ntfs {
    fn name(&self) -> &'static str {
        "NTFS"
    }

    fn deleted(&self, r: &dyn ReadAt, emit: &mut dyn FnMut(Entry)) -> io::Result<()> {
        // Duyệt toàn bộ MFT: gom thư mục (để dựng đường dẫn) và file đã xóa
        let (mut dirs, mut found) = (HashMap::new(), vec![]);
        let (mut idx, total) = (0u64, self.size / self.rs as u64);
        let mut buf = vec![0u8; CH];
        for &(l, n) in &self.mft {
            let mut o = self.off(l);
            let mut left = n.saturating_mul(self.csz);
            while left > 0 && idx < total {
                let len = left.min(CH as u64) as usize / self.rs * self.rs;
                if len == 0 {
                    break;
                }
                let b = &mut buf[..len];
                if r.read_at(o, b).is_ok() {
                    for (i, rec) in b.chunks_exact_mut(self.rs).enumerate() {
                        if idx + i as u64 >= total {
                            break;
                        }
                        let Some(x) = parse(rec) else { continue };
                        if x.dir {
                            dirs.insert(idx + i as u64, (x.parent, x.name));
                        } else if !x.used && x.size > 0 && !x.runs.is_empty() {
                            found.push(x);
                        }
                    }
                }
                idx += (len / self.rs) as u64;
                o += len as u64;
                left -= len as u64;
            }
        }
        for x in found {
            let mut ok = !self.bm.is_empty();
            let mut runs: Vec<Run> = x
                .runs
                .iter()
                .map(|&(l, n)| {
                    let len = n.saturating_mul(self.csz);
                    match l {
                        Some(l) => {
                            ok &= bits_clear(&self.bm, l, n);
                            Run { off: self.off(l), len }
                        }
                        None => Run { off: HOLE, len },
                    }
                })
                .collect();
            clip(&mut runs, x.size);
            emit(Entry { path: path(&dirs, x.parent, x.name), size: x.size, runs, ok });
        }
        Ok(())
    }
}

/// Cắt danh sách run đúng `size` byte.
fn clip(v: &mut Vec<Run>, mut size: u64) {
    v.retain_mut(|r| {
        let keep = size > 0;
        r.len = r.len.min(size);
        size -= r.len;
        keep
    });
}

fn path(d: &HashMap<u64, (u64, String)>, mut p: u64, name: String) -> String {
    let mut v = vec![name];
    for _ in 0..64 {
        if p == 5 {
            break; // record 5 = thư mục gốc
        }
        match d.get(&p) {
            Some((pp, n)) => {
                v.push(n.clone());
                p = *pp;
            }
            None => {
                v.push("?".into());
                break;
            }
        }
    }
    v.reverse();
    format!("/{}", v.join("/"))
}

/// Áp fixup rồi đọc các attribute cần dùng. None nếu không phải base record hợp lệ.
fn parse(rec: &mut [u8]) -> Option<Rec> {
    let (o, n) = (u16le(rec, 4) as usize, u16le(rec, 6) as usize);
    if &rec[..4] != b"FILE" || n < 2 || o + 2 * n > rec.len() || (n - 1) * 512 > rec.len() || u64le(rec, 32) & MASK != 0 {
        return None;
    }
    for i in 1..n {
        rec[i * 512 - 2] = rec[o + 2 * i];
        rec[i * 512 - 1] = rec[o + 2 * i + 1];
    }
    let flags = u16le(rec, 22);
    let (mut parent, mut name, mut size, mut runs) = (0, None, 0, vec![]);
    let mut p = u16le(rec, 20) as usize;
    while p + 8 <= rec.len() {
        let (t, l) = (u32le(rec, p), u32le(rec, p + 4) as usize);
        if t == u32::MAX || l < 16 || p + l > rec.len() {
            break;
        }
        let a = &rec[p..p + l];
        p += l;
        match t {
            // $FILE_NAME (resident); bỏ tên DOS 8.3 (namespace 2) nếu đã có tên khác
            0x30 if a[8] == 0 => {
                let Some(c) = a.get(u16le(a, 20) as usize..) else { continue };
                let nl = c.get(64).map_or(0, |&x| x as usize);
                if c.get(65) != Some(&2) || name.is_none() {
                    if let Some(w) = c.get(66..66 + 2 * nl) {
                        parent = u64le(c, 0) & MASK;
                        let u: Vec<u16> = w.chunks_exact(2).map(|x| u16::from_le_bytes([x[0], x[1]])).collect();
                        name = Some(String::from_utf16_lossy(&u));
                    }
                }
            }
            // $DATA không tên, non-resident, không nén/mã hóa, đoạn đầu (VCN 0)
            0x80 if a[8] == 1 && a[9] == 0 && u16le(a, 12) & 0x4001 == 0 && u64le(a, 16) == 0 => {
                size = u64le(a, 48);
                runs = a.get(u16le(a, 32) as usize..).map(decode).unwrap_or_default();
            }
            _ => {}
        }
    }
    Some(Rec { used: flags & 1 != 0, dir: flags & 2 != 0, parent, name: name?, size, runs })
}

/// Giải mã data runs: mỗi run = header (độ dài field len | field offset << 4), len, offset có dấu (delta).
fn decode(b: &[u8]) -> Runs {
    let (mut v, mut i, mut lcn) = (vec![], 0, 0i64);
    while let Some(&h) = b.get(i) {
        let (ls, os) = ((h & 15) as usize, (h >> 4) as usize);
        if h == 0 || ls == 0 || ls > 8 || os > 8 {
            break;
        }
        let Some(f) = b.get(i + 1..i + 1 + ls + os) else { break };
        i += 1 + ls + os;
        let len = le(&f[..ls]);
        if os == 0 {
            v.push((None, len));
            continue;
        }
        lcn = lcn.wrapping_add(sle(&f[ls..]));
        if lcn < 0 {
            break;
        }
        v.push((Some(lcn as u64), len));
    }
    v
}

fn le(b: &[u8]) -> u64 {
    b.iter().rev().fold(0, |a, &x| a << 8 | x as u64)
}

fn sle(b: &[u8]) -> i64 {
    let s = 64 - 8 * b.len() as u32;
    ((le(b) << s) as i64) >> s
}
