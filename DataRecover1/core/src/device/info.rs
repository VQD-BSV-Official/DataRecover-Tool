use super::Device;
use crate::fs;
use std::{fmt::Write, io, path::Path};

#[derive(Default)]
pub struct Info {
    pub path: String,
    pub size: u64,
    pub sector: usize,
    pub vendor: String,
    pub model: String,
    pub serial: String,
    pub bus: String, // ví dụ "USB 3.0 (5000 Mbps)"
    pub removable: Option<bool>,
    pub volumes: Vec<Vol>,
}

pub struct Vol {
    pub start: u64,
    pub len: u64,
    pub fs: &'static str,
}

/// Thông tin phần cứng (tùy hệ điều hành) + phân vùng/filesystem đọc từ chính thiết bị.
pub fn collect(d: &Device) -> Info {
    let mut i = Info { path: d.path().into(), size: d.size(), sector: d.sector(), ..Default::default() };
    sys(&mut i);
    if let Ok(v) = fs::volumes(d) {
        i.volumes = v.iter().map(|(p, f)| Vol { start: p.start, len: if p.len == 0 { d.size() } else { p.len }, fs: f.name() }).collect();
    }
    i
}

pub fn write_log(i: &Info, path: impl AsRef<Path>) -> io::Result<()> {
    let or = |s: &str| if s.is_empty() { "không rõ".to_owned() } else { s.to_owned() };
    let gb = |n: u64| format!("{} byte = {:.2} GB ({:.2} GiB)", n, n as f64 / 1e9, n as f64 / (1u64 << 30) as f64);
    let mut s = String::new();
    let _ = writeln!(s, "Thiết bị: {}", i.path);
    let _ = writeln!(s, "Hãng: {}", or(&i.vendor));
    let _ = writeln!(s, "Model: {}", or(&i.model));
    let _ = writeln!(s, "Serial: {}", or(&i.serial));
    let _ = writeln!(s, "Giao tiếp: {}", or(&i.bus));
    let _ = writeln!(s, "Rút được: {}", i.removable.map_or("không rõ", |r| if r { "có" } else { "không" }));
    let _ = writeln!(s, "Dung lượng: {}", gb(i.size));
    let _ = writeln!(s, "Sector: {} byte", i.sector);
    let _ = writeln!(s, "Số volume nhận dạng được: {}", i.volumes.len());
    for (n, v) in i.volumes.iter().enumerate() {
        let _ = writeln!(s, "  #{}: {} | bắt đầu {} (sector {}) | dài {}", n + 1, v.fs, v.start, v.start / 512, gb(v.len));
    }
    std::fs::write(path, s)
}

/// Danh sách thiết bị lưu trữ để GUI chọn.
#[cfg(target_os = "linux")]
pub fn list() -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir("/sys/block")
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| !["loop", "ram", "zram", "dm-", "sr", "md", "fd"].iter().any(|p| n.starts_with(p)))
        .filter(|n| std::fs::read_to_string(format!("/sys/block/{n}/size")).map_or(false, |s| s.trim() != "0"))
        .map(|n| format!("/dev/{n}"))
        .collect();
    v.sort();
    v
}

#[cfg(windows)]
pub fn list() -> Vec<String> {
    (0..32).map(|i| format!(r"\\.\PhysicalDrive{i}")).filter(|p| super::reader::open_file(p).is_ok()).collect()
}

#[cfg(not(any(target_os = "linux", windows)))]
pub fn list() -> Vec<String> {
    vec![]
}

#[cfg(target_os = "linux")]
fn sys(i: &mut Info) {
    use std::{fs::read_to_string, path::PathBuf};
    let Some(name) = Path::new(&i.path).file_name().and_then(|n| n.to_str()) else { return };
    let base = PathBuf::from("/sys/class/block").join(name);
    let rd = |p: PathBuf| read_to_string(p).map(|s| s.trim().to_owned()).unwrap_or_default();
    // Phân vùng (sdb1) thì thông tin nằm ở đĩa cha
    let dev = ["device", "../device"].iter().map(|x| base.join(x)).find(|p| p.exists());
    if let Some(d) = &dev {
        (i.vendor, i.model, i.serial) = (rd(d.join("vendor")), rd(d.join("model")), rd(d.join("serial")));
    }
    i.removable = ["removable", "../removable"].iter().map(|x| rd(base.join(x))).find(|s| !s.is_empty()).map(|s| s == "1");
    let usb = std::fs::canonicalize(&base).ok().and_then(|r| r.ancestors().find(|a| a.join("idVendor").exists()).map(PathBuf::from));
    i.bus = if let Some(u) = usb {
        if i.serial.is_empty() {
            i.serial = rd(u.join("serial"));
        }
        let sp = rd(u.join("speed")); // Mbps
        let v = match sp.as_str() {
            "1.5" => "1.0",
            "12" => "1.1",
            "480" => "2.0",
            "5000" => "3.0",
            "10000" => "3.1",
            "20000" => "3.2",
            _ => "?",
        };
        format!("USB {v} ({sp} Mbps)")
    } else if name.starts_with("nvme") {
        "NVMe".into()
    } else if name.starts_with("mmcblk") {
        "SD/MMC".into()
    } else if dev.is_some() {
        "SCSI/SATA".into()
    } else {
        String::new()
    };
}

/// Windows: IOCTL_STORAGE_QUERY_PROPERTY (STORAGE_DEVICE_DESCRIPTOR). Không lấy được tốc độ USB.
#[cfg(windows)]
fn sys(i: &mut Info) {
    let Ok(f) = super::reader::open_file(&i.path) else { return };
    let mut o = [0u8; 1024];
    let Ok(n) = super::ioctl(&f, 0x002D_1400, &[0u8; 12], &mut o) else { return };
    let o = &o[..n];
    let s = |at: usize| -> String {
        let p = fs::u32le(o, at) as usize;
        let t = o.get(p..).unwrap_or_default();
        if p == 0 { String::new() } else { String::from_utf8_lossy(&t[..t.iter().position(|&c| c == 0).unwrap_or(t.len())]).trim().to_owned() }
    };
    (i.vendor, i.model, i.serial) = (s(12), s(16), s(24));
    i.removable = o.get(10).map(|&r| r != 0);
    i.bus = match fs::u32le(o, 28) {
        7 => "USB".into(),
        0xC => "SD".into(),
        0xD => "MMC".into(),
        0x11 => "NVMe".into(),
        0xB => "SATA".into(),
        3 => "ATA".into(),
        1 => "SCSI".into(),
        0xA => "SAS".into(),
        b => format!("BusType {b}"),
    };
}

#[cfg(not(any(target_os = "linux", windows)))]
fn sys(_: &mut Info) {}
