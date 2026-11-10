//! Phân vùng + filesystem. Quét nhanh: liệt kê file đã xóa và vị trí dữ liệu.
use std::{io, mem::size_of};

pub mod exfat;
pub mod fat;
pub mod gpt;
pub mod mbr;
pub mod ntfs;

pub const SECTOR: u64 = 512;
/// `Run.off` của vùng sparse (NTFS): nội dung là toàn byte 0.
pub const HOLE: u64 = u64::MAX;

/// Đọc đủ `buf.len()` byte tại offset tuyệt đối trên thiết bị.
pub trait ReadAt {
    fn read_at(&self, off: u64, buf: &mut [u8]) -> io::Result<()>;
    /// Phân vùng đã quét tìm được (ưu tiên hơn MBR/GPT). Mặc định: không có.
    fn parts(&self) -> Vec<Part> {
        vec![]
    }
}

#[cfg(unix)]
impl ReadAt for std::fs::File {
    fn read_at(&self, off: u64, buf: &mut [u8]) -> io::Result<()> {
        std::os::unix::fs::FileExt::read_exact_at(self, buf, off)
    }
}

#[cfg(windows)]
impl ReadAt for std::fs::File {
    fn read_at(&self, mut off: u64, mut buf: &mut [u8]) -> io::Result<()> {
        use std::os::windows::fs::FileExt;
        while !buf.is_empty() {
            let n = self.seek_read(buf, off)?;
            if n == 0 {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
            off += n as u64;
            buf = &mut buf[n..];
        }
        Ok(())
    }
}

impl ReadAt for Vec<u8> {
    fn read_at(&self, off: u64, buf: &mut [u8]) -> io::Result<()> {
        let s = (off as usize)
            .checked_add(buf.len())
            .and_then(|e| self.get(off as usize..e))
            .ok_or(io::ErrorKind::UnexpectedEof)?;
        buf.copy_from_slice(s);
        Ok(())
    }
}

/// Vùng liên tục trên thiết bị, đơn vị byte. `len` = 0 nếu không rõ.
#[derive(Clone, Copy, Debug)]
pub struct Part {
    pub start: u64,
    pub len: u64,
}

/// Đoạn dữ liệu liên tục của file: offset tuyệt đối + độ dài (byte).
#[derive(Clone, Copy, Debug)]
pub struct Run {
    pub off: u64,
    pub len: u64,
}

#[derive(Debug)]
pub struct Entry {
    pub path: String,
    pub size: u64,
    pub runs: Vec<Run>,
    /// Cluster dữ liệu chưa bị cấp phát lại → khả năng cứu nguyên vẹn cao.
    pub ok: bool,
}

pub trait FileSystem {
    fn name(&self) -> &'static str;
    /// Gọi `emit` cho từng file đã xóa tìm được.
    fn deleted(&self, r: &dyn ReadAt, emit: &mut dyn FnMut(Entry)) -> io::Result<()>;
}

macro_rules! le {
    ($f:ident, $t:ty) => {
        /// Đọc little-endian; ngoài phạm vi trả 0 (dữ liệu hỏng không gây panic).
        pub fn $f(b: &[u8], o: usize) -> $t {
            b.get(o..o + size_of::<$t>())
                .map_or(0, |s| <$t>::from_le_bytes(s.try_into().unwrap()))
        }
    };
}
le!(u16le, u16);
le!(u32le, u32);
le!(u64le, u64);

/// Các bit [a, a+n) của bitmap đều bằng 0 (cluster trống).
pub fn bits_clear(bm: &[u8], a: u64, n: u64) -> bool {
    (a..a.saturating_add(n)).all(|i| bm.get((i / 8) as usize).is_some_and(|b| (b >> (i % 8)) & 1 == 0))
}

/// Nhận dạng filesystem tại `p`.
pub fn open(r: &dyn ReadAt, p: Part) -> io::Result<Option<Box<dyn FileSystem>>> {
    if let Some(f) = ntfs::open(r, p)? {
        return Ok(Some(Box::new(f)));
    }
    if let Some(f) = exfat::open(r, p)? {
        return Ok(Some(Box::new(f)));
    }
    Ok(fat::open(r, p)?.map(|f| Box::new(f) as Box<dyn FileSystem>))
}

/// Tất cả volume nhận dạng được. Thử "không có bảng phân vùng" (superfloppy) trước.
pub fn volumes(r: &dyn ReadAt) -> io::Result<Vec<(Part, Box<dyn FileSystem>)>> {
    let found = r.parts();
    if !found.is_empty() {
        return Ok(found.into_iter().filter_map(|p| open(r, p).ok().flatten().map(|f| (p, f))).collect());
    }
    let whole = Part { start: 0, len: 0 };
    if let Some(f) = open(r, whole)? {
        return Ok(vec![(whole, f)]);
    }
    let m = mbr::parse(r)?;
    let parts = if m.iter().any(|x| x.0 == 0xEE) {
        gpt::parse(r)?
    } else {
        m.into_iter().map(|x| x.1).collect()
    };
    Ok(parts
        .into_iter()
        .filter_map(|p| open(r, p).ok().flatten().map(|f| (p, f)))
        .collect())
}
