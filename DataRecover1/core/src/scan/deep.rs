use crate::{
    carve::{self, signature::{Scanner, OVERLAP}},
    device::Device,
    fs::{ReadAt, Run},
    ipc::Ipc,
    model::FoundFile,
};
use std::{ops::Range, sync::atomic::{AtomicBool, Ordering::Relaxed}};

const CHUNK: usize = 8 << 20; // bội của 4096 để đọc lớn đi thẳng, không qua cache
const ALIGN: u64 = 512;

pub fn run(dev: &Device, ipc: &mut Ipc, cancel: &AtomicBool, next_id: &mut u64, r: Range<u64>) {
    let Range { start: from, end: to } = r;
    let sc = Scanner::new(ALIGN);
    // [OVERLAP byte cuối của chunk trước][chunk mới]
    let mut buf = vec![0u8; OVERLAP + CHUNK];
    let (mut pos, mut keep, mut skip_to) = (from, 0usize, 0u64);

    while pos < to && !cancel.load(Relaxed) && !ipc.is_dead() {
        let n = CHUNK.min((to - pos) as usize);
        if dev.read_at(pos, &mut buf[keep..keep + n]).is_err() {
            break;
        }
        let end = keep + n;
        let base = pos - keep as u64;

        sc.scan(base, &buf[..end], |off, sig| {
            if off < skip_to {
                return;
            }
            let Some(c) = carve::carve(dev, off, sig) else { return };
            skip_to = off + c.size; // bỏ qua signature lồng bên trong (vd thumbnail JPEG)
            let ext = c.kind.ext();
            ipc.found(FoundFile {
                id: *next_id,
                name: format!("file_{off:x}.{ext}"),
                path: String::new(),
                kind: ext.into(),
                offset: off,
                size: c.size,
                source: "carve".into(),
                confidence: 60,
                runs: vec![Run { off, len: c.size }],
            });
            *next_id += 1;
        });

        let k = OVERLAP.min(end);
        buf.copy_within(end - k..end, 0);
        keep = k;
        pos += n as u64;
        ipc.progress(pos - from, to - from);
    }
    ipc.flush();
}
