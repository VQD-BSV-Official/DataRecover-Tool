# Recovery GUI

```
cd gui
pip install -e .
python -m app.main          # hoặc: recovery-gui
```

GUI tự tìm `recovery-core` tại `../target/release/` hoặc `../target/debug/`
(build bằng `cargo build --release` ở thư mục gốc), hoặc trong PATH.
Có thể chỉ định bằng biến môi trường `RECOVERY_CORE=/đường/dẫn/recovery-core`.

Đọc thiết bị thô cần quyền cao: chạy bằng Administrator (Windows) hoặc root/sudo (Linux).
Có thể bấm "Mở file image…" để thử với file ảnh đĩa mà không cần quyền cao.
