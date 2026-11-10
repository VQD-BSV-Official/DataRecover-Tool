//! CLI:
//!   recovery-core --list
//!   recovery-core <thiết_bị | E:> [quick|deep] [--log <đường_dẫn>]
//!
//! Lệnh qua stdin (mỗi dòng một JSON):
//!   {"cmd":"scan","mode":"quick"|"deep"[,"part":<id | "E" | "E:">]}
//!        // part: id (từ --list hoặc sự kiện parts_done) hoặc ký tự ổ; bỏ qua = cả đĩa
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

use device::{info, letters, Device};
use fs::{Part, ReadAt};
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

/// Cách chọn phân vùng: theo id hoặc theo ký tự ổ (C, E, F, H...)
#[derive(Clone)]
enum PartSel {
    Id(usize),
    Letter(char),
}

enum Cmd {
    Scan(Mode, Option<PartSel>),
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

/// "part": 2  |  "E"  |  "E:"  |  "2"
fn part_sel(v: &Value) -> Option<PartSel> {
    match v {
        Value::Number(n) => n.as_u64().map(|x| PartSel::Id(x as usize)),
        Value::String(s) => {
            let s = s.trim().trim_end_matches(':');
            let mut ch = s.chars();
            match (ch.next(), ch.next()) {
                (Some(l), None) if l.is_ascii_alphabetic() => {
                    Some(PartSel::Letter(l.to_ascii_uppercase()))
                }
                _ => s.parse().ok().map(PartSel::Id),
            }
        }
        _ => None,
    }
}

fn parse(v: &Value) -> Result<Cmd, String> {
    let bad = |m: &str| Err(m.to_owned());
    match v["cmd"].as_str() {
        Some("scan") => match v["mode"].as_str().and_then(mode) {
            Some(m) => Ok(Cmd::Scan(m, part_sel(&v["part"]))),
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
        let devs = info::list();
        let lets = letters::all(); // ký tự ổ -> (đĩa, offset)
        // Kèm phân vùng (MBR/GPT) của từng thiết bị; id/label dùng cho scan {"part": id}
        let details: Vec<Value> = devs
            .iter()
            .filter_map(|p| {
                let i = info::collect(&Device::open(p).ok()?);
                let no = letters::disk_no(p);
                let vols: Vec<_> = i.volumes.iter().enumerate().map(|(n, v)| {
                    let letter = lets
                        .iter()
                        .find(|(_, e)| Some(e.disk) == no && e.start == v.start)
                        .map(|(c, _)| c.to_string());
                    json!({
                        "id": n + 1, "label": ((b'A' + (n % 26) as u8) as char).to_string(),
                        "letter": letter, // "C", "E", ... hoặc null nếu không có ký tự ổ
                        "start": v.start, "len": v.len, "fs": v.fs,
                    })
                }).collect();
                Some(json!({"device": p, "size": i.size, "model": i.model, "volumes": vols}))
            })
            .collect();
        ipc.emit(json!({"ev": "devices", "devices": devs, "details": details}));
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
    let Some(mut path) = path else {
        ipc.emit(json!({"ev": "error", "error": "dùng: recovery-core <thiết_bị|E:> [quick|deep] [--log file] | --list"}));
        std::process::exit(2);
    };

    // "E" hoặc "E:" -> mở đĩa vật lý chứa ổ đó, nhớ ký tự để chọn đúng phân vùng
    let mut letter: Option<char> = None;
    {
        let t = path.trim_end_matches(':');
        if t.len() == 1 && t.as_bytes()[0].is_ascii_alphabetic() {
            let c = t.chars().next().unwrap().to_ascii_uppercase();
            match letters::resolve(c) {
                Some(e) => {
                    path = letters::physical(e.disk);
                    letter = Some(c);
                }
                None => {
                    ipc.emit(json!({"ev": "error", "error": format!("không tra được ổ {c}:")}));
                    std::process::exit(1);
                }
            }
        }
    }

    let dev = match Device::open(&path) {
        Ok(d) => d,
        Err(e) => {
            ipc.emit(json!({"ev": "error", "error": format!("không mở được {path}: {e}")}));
            std::process::exit(1);
        }
    };

    let inf = info::collect(&dev);

    // Bảng phân vùng MBR/GPT (id khớp với `--list`). Sau lệnh `parts` sẽ dùng kết quả quét của `parts`.
    let vols: Vec<Part> = inf.volumes.iter().map(|v| Part { start: v.start, len: v.len }).collect();
    let mut use_found = false;

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
    // Nếu chạy `recovery-core E: deep` thì tự scan đúng ổ E:
    let mut pending = first.map(|m| Cmd::Scan(m, letter.map(PartSel::Letter)));

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
                use_found = true;
            }
            Cmd::Scan(m, sel) => {
                // Đổi lựa chọn (id hoặc ký tự ổ) thành chỉ số trong dev.parts()
                let table: Vec<Part> =
                    if use_found { dev.parts().iter().copied().collect() } else { vols.clone() };
                let idx = match &sel {
                    None => None,
                    Some(PartSel::Id(i)) => Some(i.wrapping_sub(1)),
                    Some(PartSel::Letter(c)) => {
                        let found = letters::resolve(*c)
                            .and_then(|e| table.iter().position(|p| p.start == e.start));
                        match found {
                            Some(i) => Some(i),
                            None => {
                                ipc.emit(json!({"ev": "error", "error": format!("scan: ổ {c}: không nằm trên thiết bị này")}));
                                continue;
                            }
                        }
                    }
                };
                let part = match idx {
                    None => None,
                    Some(i) => match table.get(i) {
                        Some(&p) => Some(p),
                        None => {
                            ipc.emit(json!({"ev": "error", "error": "scan: không có phân vùng này (chạy parts trước)"}));
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
