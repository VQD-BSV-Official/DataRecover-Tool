"""Thanh khôi phục: chọn thư mục đích (ổ đĩa khác), nút Khôi phục, nhật ký kết quả."""
from __future__ import annotations

from pathlib import Path

from PySide6.QtCore import Signal
from PySide6.QtWidgets import (
    QFileDialog, QFrame, QHBoxLayout, QLabel, QLineEdit, QPlainTextEdit, QProgressBar, QPushButton, QVBoxLayout,
)

from app.file_model import fmt_size


class RecoverView(QFrame):
    recover_requested = Signal(str)   # thư mục đích
    cancel_requested = Signal()

    def __init__(self, parent=None):
        super().__init__(parent)
        self.setObjectName("card")
        self._count = 0
        self._busy = False
        self._ready = False

        v = QVBoxLayout(self)
        v.setContentsMargins(16, 12, 16, 12)
        v.setSpacing(8)

        self.log = QPlainTextEdit()
        self.log.setReadOnly(True)
        self.log.setMaximumHeight(90)
        self.log.hide()
        self.prog = QProgressBar()
        self.prog.setTextVisible(False)
        self.prog.setFixedHeight(8)
        self.prog.hide()
        v.addWidget(self.log)
        v.addWidget(self.prog)

        row = QHBoxLayout()
        row.setSpacing(12)
        self.lbl = QLabel("Đã chọn 0 file")
        row.addWidget(self.lbl)
        row.addStretch(1)
        row.addWidget(QLabel("Lưu sang"))
        self.out = QLineEdit(str(Path.home() / "khoi_phuc"))
        self.out.setMinimumWidth(280)
        self.out.setToolTip("Nên chọn ổ đĩa khác với thiết bị đang quét")
        row.addWidget(self.out)
        b = QPushButton("Chọn thư mục…")
        b.clicked.connect(self._browse)
        row.addWidget(b)
        self.btn = QPushButton("Khôi phục")
        self.btn.setObjectName("primary")
        self.btn.clicked.connect(self._go)
        row.addWidget(self.btn)
        v.addLayout(row)
        self._refresh()

    def _browse(self) -> None:
        d = QFileDialog.getExistingDirectory(self, "Chọn thư mục lưu file khôi phục", self.out.text())
        if d:
            self.out.setText(d)

    def _go(self) -> None:
        out = self.out.text().strip()
        if out:
            self.recover_requested.emit(out)

    # ---- API ----
    def set_selection(self, count: int, size: int) -> None:
        self._count = count
        self.lbl.setText(f"Đã chọn {count} file · {fmt_size(size)}")
        self._refresh()

    def set_ready(self, ready: bool) -> None:
        self._ready = ready
        self._refresh()

    def set_busy(self, busy: bool) -> None:
        self._busy = busy
        self._refresh()

    def _refresh(self) -> None:
        self.btn.setEnabled(self._ready and not self._busy and self._count > 0)
        self.out.setEnabled(not self._busy)

    def begin(self) -> None:
        self.log.clear()
        self.log.show()
        self.prog.setRange(0, 1000)
        self.prog.setValue(0)
        self.prog.show()

    def set_progress(self, done: int, total: int) -> None:
        self.prog.setValue(int(done * 1000 / total) if total else 0)

    def add_log(self, text: str) -> None:
        self.log.appendPlainText(text)
