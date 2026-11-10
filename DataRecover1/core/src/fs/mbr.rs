use super::{u32le, Part, ReadAt, SECTOR};
use std::io;

/// Trả về (type, phân vùng) gồm cả phân vùng logic trong extended. Rỗng nếu không có MBR.
pub fn parse(r: &dyn ReadAt) -> io::Result<Vec<(u8, Part)>> {
    let mut out = vec![];
    let mut s = [0u8; 512];
    r.read_at(0, &mut s)?;
    if s[510..] != [0x55, 0xAA] {
        return Ok(out);
    }
    let mut ext = None;
    for e in s[446..510].chunks_exact(16) {
        let (t, lba, n) = (e[4], u32le(e, 8) as u64, u32le(e, 12) as u64);
        if t == 0 || n == 0 {
            continue;
        }
        if matches!(t, 0x05 | 0x0F | 0x85) {
            ext = Some(lba);
        } else {
            out.push((t, Part { start: lba * SECTOR, len: n * SECTOR }));
        }
    }
    // EBR: entry 0 = phân vùng logic (lệch so với EBR này), entry 1 = EBR kế (lệch so với extended gốc)
    if let Some(base) = ext {
        let mut ebr = base;
        for _ in 0..128 {
            if r.read_at(ebr * SECTOR, &mut s).is_err() || s[510..] != [0x55, 0xAA] {
                break;
            }
            let (t, lba, n) = (s[450], u32le(&s, 454) as u64, u32le(&s, 458) as u64);
            if t != 0 && n != 0 {
                out.push((t, Part { start: (ebr + lba) * SECTOR, len: n * SECTOR }));
            }
            match u32le(&s, 470) as u64 {
                0 => break,
                next => ebr = base + next,
            }
        }
    }
    Ok(out)
}
