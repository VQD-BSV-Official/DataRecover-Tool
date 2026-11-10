use crate::{
    device::Device,
    fs::{ReadAt, HOLE},
    model::FoundFile,
};
use std::{
    fs::File,
    io::{self, BufWriter, Write},
    path::{Path, PathBuf},
};

pub fn write(dev: &Device, f: &FoundFile, out: &Path) -> io::Result<PathBuf> {
    let (file, path) = create(out, &sanitize(f))?;
    let mut w = BufWriter::with_capacity(1 << 20, file);
    let mut buf = vec![0u8; 1 << 20];
    let mut left = f.size; // cắt theo size thật (run cuối thường làm tròn cluster)

    for r in &f.runs {
        let (mut off, mut l) = (r.off, r.len.min(left));
        while l > 0 {
            let n = (l as usize).min(buf.len());
            if off == HOLE {
                buf[..n].fill(0); // vùng sparse NTFS
            } else {
                dev.read_at(off, &mut buf[..n])?;
                off += n as u64;
            }
            w.write_all(&buf[..n])?;
            l -= n as u64;
            left -= n as u64;
        }
        if left == 0 {
            break;
        }
    }
    w.flush()?;
    Ok(path)
}

fn sanitize(f: &FoundFile) -> String {
    let s: String = f
        .name
        .chars()
        .map(|c| if c.is_control() || "\\/:*?\"<>|".contains(c) { '_' } else { c })
        .collect();
    let s = s.trim_start_matches('.'); // chặn ".." thoát khỏi out
    if s.is_empty() { format!("file_{}", f.id) } else { s.to_owned() }
}

fn create(out: &Path, name: &str) -> io::Result<(File, PathBuf)> {
    let p = Path::new(name);
    let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or(name);
    let ext = p.extension().and_then(|s| s.to_str()).map(|e| format!(".{e}")).unwrap_or_default();
    for i in 0u32.. {
        let path = out.join(if i == 0 { name.to_owned() } else { format!("{stem}_{i}{ext}") });
        match File::create_new(&path) {
            Ok(f) => return Ok((f, path)),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
    }
    unreachable!()
}
