//! CLI:
//!   recovery-core --list
//!   recovery-core <thiết_bị> [quick|deep] [--log <đường_dẫn>]
//!
//! Lệnh qua stdin (mỗi dòng một JSON):
//!   {"cmd":"scan","mode":"quick"|"deep"[,"part":<id>]}   // part: id từ sự kiện parts_done, bỏ qua = cả đĩa
//!   {"cmd":"recover","ids":[1,2,3],"out":"D:/khoi_phuc"}
//!   {"cmd":"parts"}   // quét tìm lại phân vùng (A, B, C...), quick/deep sau đó dùng kết quả này
//!   {"cmd":"cancel"}   // dừng tác vụ đang chạy
//!   {"cmd":"quit"}

mod carve;
mod device;
mod fs;
mod ipc;
mod model;
mod recover;
mod scan;

use device::{info, Device};
use fs::ReadAt;
use ipc::Ipc;
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    io::BufRead,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering::Relaxed},
        mpsc, Arc,
    },
    thread,
};

#[derive(Clone, Copy)]
enum Mode {
    Quick,
    Deep,
}

enum Cmd {
    Scan(Mode, Option<usize>),
    Recover { ids: Vec<u64>, out: PathBuf },
    Parts,
    Bad(String),
    Quit,
}

fn mode(s: &str) -> Option<Mode> {
    match s {
        "quick" => Some(Mode::Quick),
        "deep" => Some(Mode::Deep),
        _ => None,
    }
}

fn parse(v: &Value) -> Result<Cmd, String> {
    let bad = |m: &str| Err(m.to_owned());
    match v["cmd"].as_str() {
        Some("scan") => match v["mode"].as_str().and_then(mode) {
            Some(m) => Ok(Cmd::Scan(m, v["part"].as_u64().map(|x| x as usize))),
            None => bad("scan: mode phải là quick hoặc deep"),
        },
        Some("recover") => {
            let (Some(ids), Some(out)) = (v["ids"].as_array(), v["out"].as_str()) else {
                return bad("recover: thiếu ids hoặc out");
            };
            Ok(Cmd::Recover { ids: ids.iter().filter_map(Value::as_u64).collect(), out: out.into() })
        }
        Some("parts") => Ok(Cmd::Parts),
        Some("quit") => Ok(Cmd::Quit),
        _ => bad("lệnh không hợp lệ"),
    }
}

fn main() {
    let mut ipc = Ipc::new();
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.iter().any(|a| a == "--list") {
        ipc.emit(json!({"ev": "devices", "devices": info::list()}));
        return;
    }

    let (mut path, mut first, mut log) = (None, None, PathBuf::from("Log_Info.txt"));
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "quick" => first = Some(Mode::Quick),
            "deep" => first = Some(Mode::Deep),
            "--log" => {
                if let Some(p) = it.next() {
                    log = p.into();
                }
            }
            _ => {
                if path.is_none() {
                    path = Some(a.clone());
                }
            }
        }
    }
    let Some(path) = path else {
        ipc.emit(json!({"ev": "error", "error": "dùng: recovery-core <thiết_bị> [quick|deep] [--log file] | --list"}));
        std::process::exit(2);
    };

    let dev = match Device::open(&path) {
        Ok(d) => d,
        Err(e) => {
            ipc.emit(json!({"ev": "error", "error": format!("không mở được {path}: {e}")}));
            std::process::exit(1);
        }
    };

    let inf = info::collect(&dev);
    let log_err = info::write_log(&inf, &log).err().map(|e| e.to_string());
    ipc.emit(json!({
        "ev": "ready",
        "device": inf.path, "size": inf.size, "sector": inf.sector,
        "vendor": inf.vendor, "model": inf.model, "serial": inf.serial,
        "bus": inf.bus, "removable": inf.removable,
        "volumes": inf.volumes.iter().map(|v| json!({"start": v.start, "len": v.len, "fs": v.fs})).collect::<Vec<_>>(),
        "log": log, "log_error": log_err,
    }));

    // Luồng đọc stdin: `cancel` có hiệu lực ngay, lệnh khác chuyển cho luồng chính.
    let cancel = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::channel::<Cmd>();
    {
        let cancel = cancel.clone();
        thread::spawn(move || {
            for line in std::io::stdin().lock().lines() {
                let Ok(line) = line else { break };
                let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
                if v["cmd"] == "cancel" {
                    cancel.store(true, Relaxed);
                    continue;
                }
                let c = parse(&v).unwrap_or_else(Cmd::Bad);
                if tx.send(c).is_err() {
                    break;
                }
            }
            // stdin đóng (GUI thoát): dừng việc đang chạy và thoát
            cancel.store(true, Relaxed);
            let _ = tx.send(Cmd::Quit);
        });
    }

    let mut next_id: u64;
    let mut pending = first.map(|m| Cmd::Scan(m, None));

    while !ipc.is_dead() {
        let Some(cmd) = pending.take().or_else(|| rx.recv().ok()) else { break };
        match cmd {
            Cmd::Quit => break,
            Cmd::Bad(m) => ipc.emit(json!({"ev": "error", "error": m})),
            Cmd::Parts => {
                cancel.store(false, Relaxed);
                let v = scan::parts::find(&dev, &cancel, |p| {
                    if p % (64 << 20) == 0 {
                        ipc.emit(json!({"ev": "progress", "phase": "parts", "pos": p}));
                    }
                });
                let list: Vec<_> = v
                    .iter()
                    .enumerate()
                    .map(|(i, f)| json!({"id": i + 1, "label": ((b'A' + (i % 26) as u8) as char).to_string(), "start": f.start, "len": f.len, "fs": f.fs}))
                    .collect();
                let ev = if cancel.load(Relaxed) { "parts_cancelled" } else { "parts_done" };
                ipc.emit(json!({"ev": ev, "parts": list}));
                dev.set_parts(v);
            }
            Cmd::Scan(m, sel) => {
                let part = match sel {
                    None => None,
                    Some(i) => match dev.parts().get(i.wrapping_sub(1)) {
                        Some(&p) => Some(p),
                        None => {
                            ipc.emit(json!({"ev": "error", "error": format!("scan: không có phân vùng {i} (chạy parts trước)")}));
                            continue;
                        }
                    },
                };
                let range = part.map_or(0..dev.size(), |p| {
                    let end = if p.len == 0 { dev.size() } else { (p.start + p.len).min(dev.size()) };
                    p.start..end
                });
                cancel.store(false, Relaxed);
                ipc.reset();
                next_id = 1;
                match m {
                    Mode::Quick => scan::quick::run(&dev, &mut ipc, &cancel, &mut next_id, part),
                    Mode::Deep => scan::deep::run(&dev, &mut ipc, &cancel, &mut next_id, range),
                }
                ipc.flush();
                let (count, bad) = (ipc.store().len(), dev.bad_sectors().len());
                let ev = if cancel.load(Relaxed) { "scan_cancelled" } else { "scan_done" };
                ipc.emit(json!({"ev": ev, "count": count, "bad_sectors": bad}));
            }
            Cmd::Recover { ids, out } => {
                if let Err(e) = std::fs::create_dir_all(&out) {
                    ipc.emit(json!({"ev": "error", "error": format!("không tạo được thư mục {}: {e}", out.display())}));
                    continue;
                }
                cancel.store(false, Relaxed);
                let ids: HashSet<u64> = ids.into_iter().collect();
                let files: Vec<_> = ipc.store().iter().filter(|f| ids.contains(&f.id)).cloned().collect();
                recover::run(&dev, &files, &out, &mut ipc, &cancel);
                ipc.flush();
                let ev = if cancel.load(Relaxed) { "recover_cancelled" } else { "recover_done" };
                ipc.emit(json!({"ev": ev, "requested": ids.len(), "found": files.len()}));
            }
        }
    }
    ipc.flush();
}