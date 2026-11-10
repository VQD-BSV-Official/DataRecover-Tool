"""Recovery GUI — giao diện cho recovery-core."""
from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))  # cho phép `python app/main.py`

from PySide6.QtCore import Qt
from PySide6.QtWidgets import QApplication, QFileDialog, QMainWindow, QMessageBox, QVBoxLayout, QWidget

from app.core_client import CoreClient
from app.file_model import FileFilter, FileModel
from app.views.recover_view import RecoverView
from app.views.scan_view import ScanView

STYLE = """
* { font-size: 13px; }
QMainWindow, QWidget#root, QDialog, QMessageBox { background: #14171C; }
QFrame#side { background: #1B1F26; border-right: 1px solid #2A2F38; }
QFrame#card { background: #1B1F26; border: 1px solid #2A2F38; border-radius: 8px; }
QLabel { color: #E6E8EC; }
QLabel#caption { color: #9AA3B2; font-size: 12px; font-weight: 600; }
QPushButton { padding: 7px 14px; border: 1px solid #333A46; border-radius: 8px; background: #232831; color: #E6E8EC; }
QPushButton:hover { background: #2B313C; }
QPushButton:disabled { color: #5F6775; background: #1B1F26; border-color: #2A2F38; }
QPushButton#seg:checked { background: #4C8DFF; border-color: #4C8DFF; color: #0B1220; font-weight: 600; }
QPushButton#dark { background: #E6E8EC; border-color: #E6E8EC; color: #14171C; font-weight: 500; }
QPushButton#dark:disabled { background: #3A404B; border-color: #3A404B; color: #7C8493; }
QPushButton#primary { background: #4C8DFF; border-color: #4C8DFF; color: #0B1220; font-weight: 600; padding: 8px 20px; }
QPushButton#primary:disabled { background: #2C3E5E; border-color: #2C3E5E; color: #7C8493; }
QLineEdit, QComboBox { padding: 6px 10px; border: 1px solid #333A46; border-radius: 8px; background: #14171C; color: #E6E8EC; selection-background-color: #4C8DFF; }
QComboBox QAbstractItemView { background: #1B1F26; color: #E6E8EC; selection-background-color: #2A3A57; }
QListWidget { border: 1px solid #333A46; border-radius: 8px; background: #14171C; color: #E6E8EC; }
QListWidget::item { padding: 8px 10px; }
QListWidget::item:selected { background: #2A3A57; color: #FFFFFF; }
QTableView { background: #1B1F26; color: #E6E8EC; border: 1px solid #2A2F38; border-radius: 8px; selection-background-color: #2A3A57; selection-color: #FFFFFF; }
QTableView::item { padding: 0 8px; border-bottom: 1px solid #2A2F38; }
QHeaderView::section { background: #232831; color: #9AA3B2; border: 0; border-bottom: 1px solid #2A2F38; padding: 8px; font-weight: 600; }
QTableCornerButton::section { background: #232831; border: 0; }
QProgressBar { border: 0; border-radius: 4px; background: #2A2F38; }
QProgressBar::chunk { border-radius: 4px; background: #4C8DFF; }
QPlainTextEdit { border: 1px solid #2A2F38; border-radius: 6px; background: #14171C; color: #C9CED6; font-family: monospace; font-size: 12px; }
QScrollBar:vertical { background: #14171C; width: 12px; }
QScrollBar::handle:vertical { background: #3A404B; border-radius: 5px; min-height: 30px; }
QScrollBar:horizontal { background: #14171C; height: 12px; }
QScrollBar::handle:horizontal { background: #3A404B; border-radius: 5px; min-width: 30px; }
QScrollBar::add-line, QScrollBar::sub-line { width: 0; height: 0; }
QToolTip { background: #232831; color: #E6E8EC; border: 1px solid #333A46; }
"""


class MainWindow(QMainWindow):
    def __init__(self):
        super().__init__()
        self.setWindowTitle("Recovery — Khôi phục dữ liệu")
        self.resize(1280, 800)

        self.client = CoreClient(self)
        self.model = FileModel(self)
        self.proxy = FileFilter(self)
        self.proxy.setSourceModel(self.model)
        self.state = "idle"       # idle | scan | recover
        self._rec_ok = 0
        self._rec_err = 0

        self.scan = ScanView(self.proxy)
        self.rec = RecoverView()
        root = QWidget()
        root.setObjectName("root")
        lay = QVBoxLayout(root)
        lay.setContentsMargins(0, 0, 0, 0)
        lay.setSpacing(0)
        lay.addWidget(self.scan, 1)
        bottom = QWidget()
        bl = QVBoxLayout(bottom)
        bl.setContentsMargins(16, 0, 16, 16)
        bl.addWidget(self.rec)
        lay.addWidget(bottom)
        self.setCentralWidget(root)

        s, c, r = self.scan, self.client, self.rec
        s.refresh_requested.connect(c.list_devices)
        s.device_chosen.connect(self.open_device)
        s.open_image_requested.connect(self.open_image)
        s.scan_requested.connect(self.start_scan)
        s.cancel_requested.connect(self.cancel)
        s.b_all.clicked.connect(lambda: self.model.set_checked_rows(self.proxy.visible_source_rows(), True))
        s.b_none.clicked.connect(lambda: self.model.set_checked_rows(self.proxy.visible_source_rows(), False))
        self.model.checked_changed.connect(
            lambda: r.set_selection(len(self.model.checked_ids()), self.model.checked_size()))
        r.recover_requested.connect(self.start_recover)

        c.devices.connect(s.set_devices)
        c.ready.connect(self.on_ready)
        c.found.connect(self.on_found)
        c.progress.connect(self.on_progress)
        c.scan_done.connect(lambda e: self.on_scan_end(e, False))
        c.scan_cancelled.connect(lambda e: self.on_scan_end(e, True))
        c.recovered.connect(self.on_recovered)
        c.recover_error.connect(self.on_recover_error)
        c.recover_done.connect(lambda e: self.on_recover_end(e, False))
        c.recover_cancelled.connect(lambda e: self.on_recover_end(e, True))
        c.error.connect(self.on_error)
        c.closed.connect(self.on_closed)

        c.list_devices()

    # ---- thao tác ----
    def open_device(self, path: str) -> None:
        self.model.clear()
        self.scan.reset_kinds()
        self.scan.clear_info()
        self.scan.reset_progress()
        self.scan.set_session(False)
        self.rec.set_ready(False)
        self.state = "idle"
        self.scan.set_status(f"Đang mở {path}…")
        self.client.open(path)

    def open_image(self) -> None:
        p, _ = QFileDialog.getOpenFileName(self, "Chọn file image đĩa", "", "Tất cả (*)")
        if p:
            self.open_device(p)

    def start_scan(self, mode: str) -> None:
        self.model.clear()
        self.scan.reset_kinds()
        self.state = "scan"
        self.scan.set_busy(True)
        self.rec.set_ready(False)
        self.scan.set_progress(0, 1)
        self.scan.set_status("Đang quét nhanh…" if mode == "quick" else "Đang quét sâu…")
        self.client.scan(mode)

    def start_recover(self, out: str) -> None:
        ids = self.model.checked_ids()
        if not ids:
            return
        self.state = "recover"
        self._rec_ok = self._rec_err = 0
        self.scan.set_busy(True)
        self.rec.set_busy(True)
        self.rec.begin()
        self.scan.set_status(f"Đang khôi phục {len(ids)} file…")
        self.client.recover(ids, out)

    def cancel(self) -> None:
        self.scan.set_status("Đang dừng…")
        self.client.cancel()

    # ---- event từ core ----
    def on_ready(self, ev: dict) -> None:
        self.scan.set_info(ev)
        self.scan.set_session(True)
        self.scan.set_status("Sẵn sàng. Chọn chế độ quét rồi bấm “Bắt đầu quét”.")

    def on_found(self, files: list) -> None:
        self.model.append(files)
        self.scan.add_kinds(f["kind"] for f in files)

    def on_progress(self, done: int, total: int) -> None:
        if self.state == "scan":
            self.scan.set_progress(done, total)
            self.scan.set_status(f"Đang quét… {done * 100 // total if total else 0}% · {self.model.rowCount()} file")
        elif self.state == "recover":
            self.rec.set_progress(done, total)
            self.scan.set_progress(done, total)

    def on_scan_end(self, ev: dict, cancelled: bool) -> None:
        self.state = "idle"
        self.scan.set_busy(False)
        self.rec.set_ready(True)
        if not cancelled:
            self.scan.set_progress(1, 1)
        bad = ev.get("bad_sectors", 0)
        txt = f"{'Đã dừng' if cancelled else 'Quét xong'}: {ev.get('count', 0)} file"
        self.scan.set_status(txt + (f" · {bad} sector hỏng" if bad else ""))

    def on_recovered(self, fid: int, path: str) -> None:
        self._rec_ok += 1
        self.rec.add_log(f"✓ #{fid} → {path}")

    def on_recover_error(self, fid: int, err: str) -> None:
        self._rec_err += 1
        self.rec.add_log(f"✗ #{fid}: {err}")

    def on_recover_end(self, ev: dict, cancelled: bool) -> None:
        self.state = "idle"
        self.scan.set_busy(False)
        self.rec.set_busy(False)
        self.scan.set_status(
            f"{'Đã dừng' if cancelled else 'Khôi phục xong'}: {self._rec_ok} thành công, {self._rec_err} lỗi")
        missing = ev.get("requested", 0) - ev.get("found", 0)
        if missing > 0:
            self.rec.add_log(f"! {missing} file không còn trong kết quả quét")

    def on_error(self, msg: str) -> None:
        if self.state == "recover":
            self.rec.add_log(f"! {msg}")
        QMessageBox.warning(self, "Lỗi", msg)
        if not self.client.is_open():
            self.on_closed()

    def on_closed(self) -> None:
        self.state = "idle"
        self.scan.set_busy(False)
        self.scan.set_session(False)
        self.rec.set_busy(False)
        self.rec.set_ready(False)
        self.scan.set_status("Chưa mở thiết bị")

    def closeEvent(self, e) -> None:
        self.client.shutdown()
        super().closeEvent(e)


def main() -> None:
    app = QApplication(sys.argv)
    app.setStyleSheet(STYLE)
    w = MainWindow()
    w.show()
    sys.exit(app.exec())


if __name__ == "__main__":
    main()
