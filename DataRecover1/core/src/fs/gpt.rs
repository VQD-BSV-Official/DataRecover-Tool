use super::{u32le, u64le, Part, ReadAt, SECTOR};
use std::io;

/// Đọc GPT chính (LBA 1). Rỗng nếu thiếu chữ ký.
pub fn parse(r: &dyn ReadAt) -> io::Result<Vec<Part>> {
    let mut h = [0u8; 512];
    r.read_at(SECTOR, &mut h)?;
    let (lba, n, sz) = (u64le(&h, 72), u32le(&h, 80) as usize, u32le(&h, 84) as usize);
    if &h[..8] != b"EFI PART" || !(128..=1024).contains(&sz) || n == 0 {
        return Ok(vec![]);
    }
    let mut t = vec![0u8; n.min(1024) * sz];
    r.read_at(lba.saturating_mul(SECTOR), &mut t)?;
    Ok(t.chunks_exact(sz)
        .filter(|e| e[..16] != [0; 16])
        .filter_map(|e| {
            let (a, b) = (u64le(e, 32), u64le(e, 40));
            (b >= a).then(|| Part { start: a.saturating_mul(SECTOR), len: (b - a).saturating_add(1).saturating_mul(SECTOR) })
        })
        .collect())
}
