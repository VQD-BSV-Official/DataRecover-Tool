use crate::fs::{Part, ReadAt};
use std::{collections::BTreeSet, fs::File, io, sync::Mutex};

const BLK: u64 = 1 << 20; // khối cache
const AL: u64 = 4096; // căn chỉnh khi đọc trực tiếp, đủ cho sector 512 và 4Kn

/// Thiết bị đọc thô: luôn đọc căn chỉnh, có cache, sector lỗi được điền 0 và ghi nhận.
pub struct Device {
    inner: Box<dyn ReadAt + Send + Sync>,
    size: u64,
    ss: usize,
    path: String,
    cache: Mutex<(u64, Vec<u8>)>,
    bad: Mutex<BTreeSet<u64>>,
    parts: Mutex<Vec<Part>>, // phân vùng do scan::parts tìm được
}

pub fn open_file(path: &str) -> io::Result<File> {
    let mut o = std::fs::OpenOptions::new();
    o.read(true);
    #[cfg(windows)]
    std::os::windows::fs::OpenOptionsExt::share_mode(&mut o, 3); // vẫn mở được khi volume đang mount
    o.open(path)
}

#[cfg(unix)]
fn raw_len(f: &File) -> io::Result<u64> {
    use std::io::{Seek, SeekFrom};
    let mut f = f;
    f.seek(SeekFrom::End(0))
}

#[cfg(windows)]
fn raw_len(f: &File) -> io::Result<u64> {
    let mut o = [0u8; 8];
    super::ioctl(f, 0x0007_405C, &[], &mut o)?; // IOCTL_DISK_GET_LENGTH_INFO
    Ok(u64::from_le_bytes(o))
}

impl Device {
    /// `path`: `/dev/sdb`, `\\.\PhysicalDrive2` hoặc file image.
    pub fn open(path: &str) -> io::Result<Self> {
        let f = open_file(path).map_err(|e| {
            io::Error::new(e.kind(), format!("OPEN_DEVICE failed: {e}"))
        })?;

        // PhysicalDrive: lấy dung lượng bằng IOCTL trước.
        // File image: nếu IOCTL không hỗ trợ thì dùng metadata().
        let size = match raw_len(&f) {
            Ok(n) if n > 0 => n,
            _ => {
                f.metadata()
                    .map_err(|e| {
                        io::Error::new(
                            e.kind(),
                            format!("GET_METADATA failed: {e}")
                        )
                    })?
                    .len()
            }
        };

        if size == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Device capacity is zero"
            ));
        }

        Ok(Self::new(Box::new(f), size, path))
    }

    pub fn new(inner: Box<dyn ReadAt + Send + Sync>, size: u64, path: &str) -> Self {
        // Thiết bị 4Kn từ chối đọc 512 byte → dùng 4096 làm đơn vị thử lại
        let ss = if inner.read_at(0, &mut [0u8; 512]).is_ok() { 512 } else { 4096 };
        Self { inner, size, ss, path: path.into(), cache: Mutex::new((u64::MAX, vec![])), bad: Mutex::default(), parts: Mutex::default() }
    }

    pub fn size(&self) -> u64 {
        self.size
    }

    pub fn sector(&self) -> usize {
        self.ss
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn set_parts(&self, v: Vec<crate::scan::parts::Found>) {
        *self.parts.lock().unwrap() = v.into_iter().map(|f| Part { start: f.start, len: f.len }).collect();
    }

    /// Offset các sector không đọc được tính đến hiện tại.
    pub fn bad_sectors(&self) -> Vec<u64> {
        self.bad.lock().unwrap().iter().copied().collect()
    }

    /// Đọc `buf` (offset phải chia hết `ss`). Lỗi → thử lại từng sector, sector hỏng điền 0.
    fn safe(&self, off: u64, buf: &mut [u8]) {
        if self.inner.read_at(off, buf).is_ok() {
            return;
        }
        for (i, ch) in buf.chunks_mut(self.ss).enumerate() {
            let o = off + (i * self.ss) as u64;
            if (0..2).all(|_| self.inner.read_at(o, ch).is_err()) {
                ch.fill(0);
                self.bad.lock().unwrap().insert(o);
            }
        }
    }
}

impl ReadAt for Device {
    fn parts(&self) -> Vec<Part> {
        self.parts.lock().unwrap().clone()
    }

    fn read_at(&self, off: u64, buf: &mut [u8]) -> io::Result<()> {
        let len = buf.len() as u64;
        if off.checked_add(len).map_or(true, |e| e > self.size) {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        // Đọc lớn đã căn chỉnh: bỏ qua cache, không copy thừa
        if len >= BLK && off % AL == 0 && len % AL == 0 {
            self.safe(off, buf);
            return Ok(());
        }
        let mut g = self.cache.lock().unwrap();
        let (key, blk) = &mut *g;
        let (mut o, mut b) = (off, buf);
        while !b.is_empty() {
            let base = o / BLK * BLK;
            if *key != base {
                blk.resize(BLK.min(self.size - base) as usize, 0);
                *key = u64::MAX;
                self.safe(base, blk);
                *key = base;
            }
            let s = (o - base) as usize;
            let k = b.len().min(blk.len() - s);
            let (head, tail) = b.split_at_mut(k);
            head.copy_from_slice(&blk[s..s + k]);
            o += k as u64;
            b = tail;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Giả lập 1 sector hỏng tại `.1`.
    struct Faulty(Vec<u8>, u64);
    impl ReadAt for Faulty {
        fn read_at(&self, off: u64, buf: &mut [u8]) -> io::Result<()> {
            if off < self.1 + 512 && off + buf.len() as u64 > self.1 {
                return Err(io::ErrorKind::Other.into());
            }
            self.0.read_at(off, buf)
        }
    }

    #[test]
    fn bad_sector_and_boundaries() {
        let data: Vec<u8> = (0..5_000_000u32).map(|i| (i.wrapping_mul(7) + 1) as u8).collect();
        let bad = 1_499_648u64; // chia hết 512, nằm trong khối cache thứ 2
        let mut exp = data.clone();
        exp[bad as usize..bad as usize + 512].fill(0);
        let d = Device::new(Box::new(Faulty(data.clone(), bad)), data.len() as u64, "t");
        for (off, len) in [(0u64, 512usize), (1_048_000, 2000), (1_499_000, 2000), (4_096, 2 << 20), (1 << 20, 3 << 20), (4_999_000, 1000)] {
            let mut b = vec![1u8; len];
            d.read_at(off, &mut b).unwrap();
            assert_eq!(b, exp[off as usize..off as usize + len], "off={off} len={len}");
        }
        assert_eq!(d.bad_sectors(), vec![bad]);
        assert!(d.read_at(4_999_999, &mut [0u8; 2]).is_err());
    }
}

/// Cho phép module `carve` đọc trực tiếp từ `Device`.
impl crate::carve::Src for Device {
    fn len(&self) -> u64 {
        self.size
    }

    fn read_at(&self, off: u64, buf: &mut [u8]) -> io::Result<usize> {
        let n = (buf.len() as u64).min(self.size.saturating_sub(off)) as usize;
        ReadAt::read_at(self, off, &mut buf[..n])?;
        Ok(n)
    }
}
