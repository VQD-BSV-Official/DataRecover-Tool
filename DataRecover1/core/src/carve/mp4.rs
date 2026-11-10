use super::{Kind, Src};

/// `off` = đầu box ftyp (đã trừ 4 byte size). Duyệt box top-level.
pub fn probe(s: &impl Src, off: u64) -> Option<(Kind, u64)> {
    let kind = if &s.rd::<4>(off + 8)? == b"qt  " { Kind::Mov } else { Kind::Mp4 };
    let (mut p, mut moov, mut mdat) = (off, false, false);
    let end = s.len();
    while let Some(h) = s.rd::<8>(p) {
        let t: [u8; 4] = h[4..].try_into().unwrap();
        match &t {
            b"moov" => moov = true,
            b"mdat" => mdat = true,
            b"ftyp" | b"free" | b"skip" | b"wide" | b"pnot" | b"uuid" | b"meta" | b"udta"
            | b"moof" | b"mfra" | b"sidx" | b"styp" | b"junk" => {}
            _ => break,
        }
        let mut sz = u32::from_be_bytes(h[..4].try_into().unwrap()) as u64;
        if sz == 1 { sz = u64::from_be_bytes(s.rd(p + 8)?); }
        if sz < 8 || p + sz > end || (p == off && sz > 1024) { break; } // sz=0 (tới EOF) không xác định được
        p += sz;
    }
    (moov && mdat).then_some((kind, p - off))
}
