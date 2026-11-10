use crate::{device::Device, fs::{self, Part}, ipc::Ipc, model::FoundFile};
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};

pub fn run(dev: &Device, ipc: &mut Ipc, cancel: &AtomicBool, next_id: &mut u64, part: Option<Part>) {
    let vols = match part {
        Some(p) => match fs::open(dev, p) {
            Ok(Some(f)) => vec![(p, f)],
            _ => return,
        },
        None => match fs::volumes(dev) {
            Ok(v) => v,
            Err(_) => return,
        },
    };
    for (_, v) in vols {
        if cancel.load(Relaxed) || ipc.is_dead() {
            break;
        }
        let _ = v.deleted(dev, &mut |e| {
            if cancel.load(Relaxed) {
                return;
            }
            ipc.found(FoundFile::from_entry(*next_id, e));
            *next_id += 1;
        });
    }
    ipc.flush();
}
