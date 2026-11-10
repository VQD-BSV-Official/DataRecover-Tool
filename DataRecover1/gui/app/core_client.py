"""Chạy recovery-core như tiến trình con, đọc event NDJSON từ stdout, gửi lệnh qua stdin."""
from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

from PySide6.QtCore import QObject, QProcess, Signal


def find_core() -> str | None:
    exe = "recovery-core.exe" if sys.platform == "win32" else "recovery-core"

    # 1. Biến môi trường (ưu tiên cao nhất)
    env = os.environ.get("RECOVERY_CORE")
    if env and Path(env).is_file():
        return env

    # 2. Cạnh Recovery.exe (đã đóng gói) hoặc cạnh file này (chạy bằng python)
    here = Path(sys.executable).parent if getattr(sys, "frozen", False) else Path(__file__).resolve().parent
    p = here / exe
    if p.is_file():
        return str(p)

    # 3. Thư mục build của Cargo: recovery/target/release|debug
    root = Path(__file__).resolve().parents[2]
    for sub in ("release", "debug"):
        p = root / "target" / sub / exe
        if p.is_file():
            return str(p)

    # 4. PATH hệ thống
    return shutil.which("recovery-core")


class CoreClient(QObject):
    devices = Signal(list)           # danh sách đường dẫn thiết bị
    ready = Signal(dict)             # thông tin thiết bị sau khi mở
    found = Signal(list)             # batch file tìm thấy (list[dict])
    progress = Signal(int, int)      # done, total
    scan_done = Signal(dict)         # {count, bad_sectors}
    scan_cancelled = Signal(dict)
    recovered = Signal(int, str)     # id, đường dẫn đã ghi
    recover_error = Signal(int, str)
    recover_done = Signal(dict)      # {requested, found}
    recover_cancelled = Signal(dict)
    error = Signal(str)
    closed = Signal()                # tiến trình core đã kết thúc

    def __init__(self, parent=None):
        super().__init__(parent)
        self._p: QProcess | None = None
        self._buf = b""
        self._err = b""
        self._stopping = False

    # ---- vòng đời ----
    def is_open(self) -> bool:
        return self._p is not None and self._p.state() != QProcess.ProcessState.NotRunning

    def list_devices(self) -> None:
        core = find_core()
        if not core:
            self.error.emit("Không tìm thấy recovery-core. Hãy chạy `cargo build --release` hoặc đặt RECOVERY_CORE.")
            return
        flags = getattr(subprocess, "CREATE_NO_WINDOW", 0)
        try:
            r = subprocess.run([core, "--list"], capture_output=True, timeout=15,
                            stdin=subprocess.DEVNULL, creationflags=flags)
        except Exception as e:  # noqa: BLE001
            self.error.emit(f"Không liệt kê được thiết bị: {e}")
            return
        out = r.stdout.decode("utf-8", "replace")
        for line in out.splitlines():
            try:
                v = json.loads(line)
            except ValueError:
                continue
            if v.get("ev") == "devices":
                devs = v.get("devices", [])
                if not devs:
                    self.error.emit(
                        f"Core trả về danh sách rỗng.\nCore: {core}\n"
                        f"Output: {out.strip()}\nStderr: {r.stderr.decode('utf-8', 'replace').strip()}"
                    )
                self.devices.emit(devs)
                return
        self.error.emit(f"Không đọc được output của core.\nCore: {core}\nOutput: {out.strip()}")
        self.devices.emit([])



    def open(self, path: str) -> None:
        self.shutdown()
        core = find_core()
        if not core:
            self.error.emit("Không tìm thấy recovery-core. Hãy chạy `cargo build --release` hoặc đặt RECOVERY_CORE.")
            return
        self._buf, self._err, self._stopping = b"", b"", False
        p = QProcess(self)
        p.setProgram(core)
        p.setArguments([path])
        p.readyReadStandardOutput.connect(self._on_out)
        p.readyReadStandardError.connect(lambda: self._on_err(p))
        p.finished.connect(lambda code, _st, pp=p: self._on_finished(pp, code))
        p.errorOccurred.connect(lambda _e, pp=p: self._on_proc_error(pp))
        self._p = p
        p.start()

    def shutdown(self) -> None:
        p, self._p = self._p, None
        if p is None:
            return
        self._stopping = True
        if p.state() != QProcess.ProcessState.NotRunning:
            p.write(b'{"cmd":"quit"}\n')
            p.closeWriteChannel()
            if not p.waitForFinished(1500):
                p.kill()
                p.waitForFinished(1000)
        p.deleteLater()

    # ---- lệnh ----
    def _send(self, obj: dict) -> None:
        if self.is_open():
            self._p.write(json.dumps(obj, ensure_ascii=False).encode("utf-8") + b"\n")

    def scan(self, mode: str) -> None:
        self._send({"cmd": "scan", "mode": mode})

    def recover(self, ids: list[int], out: str) -> None:
        self._send({"cmd": "recover", "ids": ids, "out": out})

    def cancel(self) -> None:
        self._send({"cmd": "cancel"})

    # ---- đọc event ----
    def _on_out(self) -> None:
        if self._p is None:
            return
        self._buf += bytes(self._p.readAllStandardOutput())
        while b"\n" in self._buf:
            line, self._buf = self._buf.split(b"\n", 1)
            if line.strip():
                try:
                    self._dispatch(json.loads(line.decode("utf-8")))
                except ValueError:
                    pass

    def _on_err(self, p: QProcess) -> None:
        self._err += bytes(p.readAllStandardError())

    def _dispatch(self, v: dict) -> None:
        ev = v.get("ev")
        if ev == "ready":
            self.ready.emit(v)
        elif ev == "found":
            self.found.emit(v.get("files", []))
        elif ev == "progress":
            self.progress.emit(int(v.get("done", 0)), int(v.get("total", 0)))
        elif ev == "scan_done":
            self.scan_done.emit(v)
        elif ev == "scan_cancelled":
            self.scan_cancelled.emit(v)
        elif ev == "recovered":
            self.recovered.emit(int(v.get("id", 0)), str(v.get("path", "")))
        elif ev == "recover_error":
            self.recover_error.emit(int(v.get("id", 0)), str(v.get("error", "")))
        elif ev == "recover_done":
            self.recover_done.emit(v)
        elif ev == "recover_cancelled":
            self.recover_cancelled.emit(v)
        elif ev == "error":
            self.error.emit(str(v.get("error", "lỗi không rõ")))

    def _on_finished(self, p: QProcess, code: int) -> None:
        if p is not self._p or self._stopping:
            return
        self._on_out()
        msg = self._err.decode("utf-8", "replace").strip()
        if code != 0 and msg:
            self.error.emit(msg)
        self.closed.emit()

    def _on_proc_error(self, p: QProcess) -> None:
        if p is self._p and not self._stopping and p.state() == QProcess.ProcessState.NotRunning \
                and p.exitStatus() != QProcess.ExitStatus.NormalExit:
            self.error.emit(f"Không chạy được recovery-core: {p.errorString()}")
            self.closed.emit()
