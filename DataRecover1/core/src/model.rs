use crate::fs::{Entry, Run, HOLE};
use serde::Serialize;

/// File tìm thấy (gửi sang GUI qua NDJSON).
#[derive(Clone, Debug, Default, Serialize)]
pub struct FoundFile {
    pub id: u64,
    pub name: String,
    pub path: String,       // đường dẫn trong filesystem ("" nếu carve)
    pub kind: String,       // đuôi file: jpg, mp4, ...
    pub offset: u64,        // offset đầu tiên của dữ liệu
    pub size: u64,
    pub source: String,     // "fs" | "carve"
    pub confidence: u8,     // 0..=100
    /// Các đoạn dữ liệu để khôi phục (chỉ dùng nội bộ, không gửi cho GUI).
    #[serde(skip)]
    pub runs: Vec<Run>,
}

impl FoundFile {
    /// Dựng từ kết quả quét nhanh của filesystem. `id` do bên gọi cấp.
    pub fn from_entry(id: u64, e: Entry) -> Self {
        let name = e.path.rsplit('/').next().unwrap_or("").to_owned();
        let kind = name
            .rsplit_once('.')
            .map(|(_, x)| x.to_ascii_lowercase())
            .filter(|x| !x.is_empty() && x.len() <= 8)
            .unwrap_or_else(|| "unknown".into());
        let offset = e.runs.iter().find(|r| r.off != HOLE).map_or(0, |r| r.off);
        FoundFile {
            id,
            name,
            path: e.path,
            kind,
            offset,
            size: e.size,
            source: "fs".into(),
            confidence: if e.ok { 90 } else { 30 },
            runs: e.runs,
        }
    }
}