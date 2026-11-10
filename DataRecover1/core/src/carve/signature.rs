use super::Sig;
use aho_corasick::AhoCorasick;

/// Chunk kế tiếp phải giữ lại OVERLAP byte cuối của chunk trước.
/// Mọi pattern dài 4 byte nên không bị báo trùng.
pub const OVERLAP: usize = 3;

const PATS: [(&[u8; 4], Sig); 10] = [
    (b"\xFF\xD8\xFF\xE0", Sig::Jpeg),
    (b"\xFF\xD8\xFF\xE1", Sig::Jpeg),
    (b"\xFF\xD8\xFF\xE2", Sig::Jpeg),
    (b"\xFF\xD8\xFF\xE8", Sig::Jpeg),
    (b"\xFF\xD8\xFF\xDB", Sig::Jpeg),
    (b"\xFF\xD8\xFF\xED", Sig::Jpeg),
    (b"\xFF\xD8\xFF\xEE", Sig::Jpeg),
    (b"II*\0", Sig::TiffLe),
    (b"MM\0*", Sig::TiffBe),
    (b"ftyp", Sig::Ftyp), // header thật nằm trước 4 byte
];

pub struct Scanner { ac: AhoCorasick, align: u64 }

impl Scanner {
    /// `align`: bội số offset hợp lệ của header (512/4096 khi quét theo sector, 1 = không giới hạn).
    pub fn new(align: u64) -> Self {
        Self { ac: AhoCorasick::new(PATS.iter().map(|p| p.0)).unwrap(), align: align.max(1) }
    }

    /// `base`: offset tuyệt đối của buf[0]. Gọi `f(offset_bắt_đầu_file, sig)`.
    pub fn scan(&self, base: u64, buf: &[u8], mut f: impl FnMut(u64, Sig)) {
        for m in self.ac.find_iter(buf) {
            let sig = PATS[m.pattern().as_usize()].1;
            let pos = base + m.start() as u64;
            let start = if sig == Sig::Ftyp {
                match pos.checked_sub(4) { Some(x) => x, None => continue }
            } else { pos };
            if start % self.align == 0 { f(start, sig); }
        }
    }
}
