use crate::model::FoundFile;
use serde_json::{json, Value};
use std::{
    io::{self, BufWriter, Write},
    time::{Duration, Instant},
};

const BATCH: usize = 256;
const EVERY: Duration = Duration::from_millis(100);

/// Phát event NDJSON (mỗi dòng một JSON) ra stdout.
/// `found` được gom batch; `progress` bị giới hạn tần suất.
pub struct Ipc {
    out: BufWriter<io::Stdout>,
    batch: Vec<FoundFile>,
    store: Vec<FoundFile>, // toàn bộ file đã báo, để recover tra theo id
    last_batch: Instant,
    last_prog: Instant,
    dead: bool, // stdout đã đóng (GUI thoát)
}

impl Ipc {
    pub fn new() -> Self {
        let now = Instant::now();
        Self {
            out: BufWriter::new(io::stdout()),
            batch: Vec::with_capacity(BATCH),
            store: vec![],
            last_batch: now,
            last_prog: now,
            dead: false,
        }
    }

    pub fn is_dead(&self) -> bool {
        self.dead
    }

    /// File đã tìm thấy kể từ lần `reset` gần nhất (cả những file chưa flush).
    pub fn store(&self) -> &[FoundFile] {
        &self.store
    }

    pub fn reset(&mut self) {
        self.batch.clear();
        self.store.clear();
    }

    fn line(&mut self, v: &Value) {
        let ok = serde_json::to_writer(&mut self.out, v).is_ok() && self.out.write_all(b"\n").is_ok();
        if !ok {
            self.dead = true;
        }
    }

    fn flush_batch(&mut self) {
        if self.batch.is_empty() {
            return;
        }
        let ev = json!({"ev": "found", "files": self.batch});
        self.line(&ev);
        self.store.append(&mut self.batch);
        self.last_batch = Instant::now();
    }

    /// Event rời rạc: đẩy batch đang chờ trước để giữ đúng thứ tự, rồi flush ngay.
    pub fn emit(&mut self, v: Value) {
        self.flush_batch();
        self.line(&v);
        self.flush_out();
    }

    pub fn found(&mut self, f: FoundFile) {
        self.batch.push(f);
        if self.batch.len() >= BATCH || self.last_batch.elapsed() >= EVERY {
            self.flush_batch();
            self.flush_out();
        }
    }

    pub fn progress(&mut self, done: u64, total: u64) {
        if done < total && self.last_prog.elapsed() < EVERY {
            return;
        }
        self.last_prog = Instant::now();
        self.emit(json!({"ev": "progress", "done": done, "total": total}));
    }

    pub fn flush(&mut self) {
        self.flush_batch();
        self.flush_out();
    }

    fn flush_out(&mut self) {
        if self.out.flush().is_err() {
            self.dead = true;
        }
    }
}