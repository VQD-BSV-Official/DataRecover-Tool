pub mod writer;

use crate::{device::Device, ipc::Ipc, model::FoundFile};
use serde_json::json;
use std::{
    path::Path,
    sync::atomic::{AtomicBool, Ordering::Relaxed},
};

pub fn run(dev: &Device, files: &[FoundFile], out: &Path, ipc: &mut Ipc, cancel: &AtomicBool) {
    for (i, f) in files.iter().enumerate() {
        if cancel.load(Relaxed) {
            break;
        }
        let ev = match writer::write(dev, f, out) {
            Ok(p) => json!({"ev": "recovered", "id": f.id, "path": p}),
            Err(e) => json!({"ev": "recover_error", "id": f.id, "error": e.to_string()}),
        };
        ipc.emit(ev);
        ipc.progress(i as u64 + 1, files.len() as u64);
    }
    ipc.flush();
}
