use std::io;

mod jpeg;
mod mp4;
pub mod signature;
mod tiff;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind { Jpeg, Cr2, Nef, Arw, Mp4, Mov }

impl Kind {
    pub fn ext(self) -> &'static str {
        match self {
            Kind::Jpeg => "jpg", Kind::Cr2 => "cr2", Kind::Nef => "nef",
            Kind::Arw => "arw", Kind::Mp4 => "mp4", Kind::Mov => "mov",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sig { Jpeg, TiffLe, TiffBe, Ftyp }

#[derive(Debug)]
pub struct Carved { pub kind: Kind, pub size: u64 }

/// Nguồn đọc ngẫu nhiên (device::reader implement trait này).
pub trait Src {
    fn len(&self) -> u64;
    fn read_at(&self, off: u64, buf: &mut [u8]) -> io::Result<usize>;

    fn read_exact_at(&self, mut off: u64, mut buf: &mut [u8]) -> Option<()> {
        while !buf.is_empty() {
            match self.read_at(off, buf) {
                Ok(n) if n > 0 => { off += n as u64; buf = &mut buf[n..]; }
                _ => return None,
            }
        }
        Some(())
    }
    fn rd<const N: usize>(&self, off: u64) -> Option<[u8; N]> where Self: Sized {
        let mut b = [0; N];
        self.read_exact_at(off, &mut b)?;
        Some(b)
    }
    fn read_vec(&self, off: u64, n: usize) -> Option<Vec<u8>> {
        let mut v = vec![0; n];
        self.read_exact_at(off, &mut v)?;
        Some(v)
    }
}

/// Xác thực signature tại `off` và tính kích thước file.
pub fn carve(s: &impl Src, off: u64, sig: Sig) -> Option<Carved> {
    let (kind, size) = match sig {
        Sig::Jpeg => (Kind::Jpeg, jpeg::size(s, off)?),
        Sig::TiffLe => tiff::probe(s, off, false)?,
        Sig::TiffBe => tiff::probe(s, off, true)?,
        Sig::Ftyp => mp4::probe(s, off)?,
    };
    Some(Carved { kind, size })
}
