use crate::{device::Device, fs::ReadAt};
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};

pub struct Found {
    pub start: u64,
    pub len: u64,
    pub fs: &'static str,
}

const CH: usize = 1 << 20;

/// `s`: đúng 1 sector 512B tại offset `o`. Bắt boot sector NTFS/exFAT/FAT và superblock ext (start = o-1024).
fn probe(s: &[u8], o: u64, size: u64) -> Option<Found> {
    let le16 = |i: usize| u16::from_le_bytes([s[i], s[i + 1]]) as u64;
    let le32 = |i: usize| u32::from_le_bytes(s[i..i + 4].try_into().unwrap()) as u64;
    let le64 = |i: usize| u64::from_le_bytes(s[i..i + 8].try_into().unwrap());
    let boot = s[510] == 0x55 && s[511] == 0xAA;
    let bps = le16(0x0B);
    let bps_ok = matches!(bps, 512 | 1024 | 2048 | 4096);

    let (fs, start, len) = if boot && bps_ok && &s[3..11] == b"NTFS    " {
        let len = (le64(0x28) + 1) * bps;
        // phân vùng chạy quá đĩa => đây là boot sector dự phòng ở cuối volume
        let start = if o + len > size { (o + bps).checked_sub(len)? } else { o };
        ("ntfs", start, len)
    } else if boot && &s[3..11] == b"EXFAT   " {
        ("exfat", o, le64(0x48) << s[0x6C].min(12))
    } else if boot && bps_ok && &s[0x52..0x5A] == b"FAT32   " {
        ("fat32", o, le32(0x20) * bps)
    } else if boot && bps_ok && &s[0x36..0x3E] == b"FAT16   " {
        ("fat16", o, (if le16(0x13) != 0 { le16(0x13) } else { le32(0x20) }) * bps)
    } else if le16(0x38) == 0xEF53 && le16(0x5A) == 0 && le32(0x18) <= 6 && o >= 1024 {
        ("ext", o - 1024, le32(4) << (10 + le32(0x18)))
    } else {
        return None;
    };
    (len > 0 && start + len <= size).then_some(Found { start, len, fs })
}

pub fn find(d: &Device, cancel: &AtomicBool, mut prog: impl FnMut(u64)) -> Vec<Found> {
    let (size, mut v, mut end, mut buf) = (d.size(), vec![], 0u64, vec![0u8; CH]);
    let mut off = 0;
    while off + 512 <= size && !cancel.load(Relaxed) {
        let n = (CH as u64).min(size - off) as usize / 512 * 512;
        if d.read_at(off, &mut buf[..n]).is_err() {
            break;
        }
        for i in (0..n).step_by(512) {
            let o = off + i as u64;
            if o < end {
                continue; // bản sao/backup nằm trong phân vùng đã tìm thấy
            }
            if let Some(f) = probe(&buf[i..i + 512], o, size) {
                end = f.start + f.len;
                v.push(f);
            }
        }
        off += n as u64;
        prog(off);
    }
    v
}
