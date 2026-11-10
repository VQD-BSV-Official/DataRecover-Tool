"""Màn hình quét: danh sách thiết bị, thông tin, chế độ quét, tiến trình, bảng kết quả."""
from __future__ import annotations

from PySide6.QtCore import Qt, Signal
from PySide6.QtWidgets import (
    QAbstractItemView, QButtonGroup, QComboBox, QFrame, QHBoxLayout, QHeaderView, QLabel, QLineEdit,
    QListWidget, QProgressBar, QPushButton, QTableView, QVBoxLayout, QWidget,
)

from app.file_model import FileFilter, fmt_size


def _caption(text: str) -> QLabel:
    l = QLabel(text)
    l.setObjectName("caption")
    return l


class ScanView(QWidget):
    device_chosen = Signal(str)
    refresh_requested = Signal()
    open_image_requested = Signal()
    scan_requested = Signal(str)   # "quick" | "deep"
    cancel_requested = Signal()

    def __init__(self, proxy: FileFilter, parent=None):
        super().__init__(parent)
        self._proxy = proxy
        self._kinds: set[str] = set()
        self._session = False
        self._busy = False

        root = QHBoxLayout(self)
        root.setContentsMargins(0, 0, 0, 0)
        root.setSpacing(0)

        # ---- cột trái ----
        side = QFrame()
        side.setObjectName("side")
        side.setFixedWidth(280)
        sl = QVBoxLayout(side)
        sl.setContentsMargins(16, 16, 16, 16)
        sl.setSpacing(10)
        sl.addWidget(_caption("THIẾT BỊ"))
        self.devices = QListWidget()
        self.devices.setMaximumHeight(170)
        self.devices.itemClicked.connect(lambda it: self.device_chosen.emit(it.data(Qt.ItemDataRole.UserRole)))
        sl.addWidget(self.devices)
        row = QHBoxLayout()
        b1, b2 = QPushButton("Làm mới"), QPushButton("Mở file image…")
        b1.clicked.connect(self.refresh_requested)
        b2.clicked.connect(self.open_image_requested)
        row.addWidget(b1)
        row.addWidget(b2)
        sl.addLayout(row)
        sl.addSpacing(6)
        sl.addWidget(_caption("THÔNG TIN"))
        self.info = QLabel("Chưa mở thiết bị")
        self.info.setWordWrap(True)
        self.info.setTextInteractionFlags(Qt.TextInteractionFlag.TextSelectableByMouse)
        self.info.setAlignment(Qt.AlignmentFlag.AlignTop | Qt.AlignmentFlag.AlignLeft)
        sl.addWidget(self.info, 1)
        self.log_lbl = QLabel("")
        self.log_lbl.setObjectName("caption")
        self.log_lbl.setWordWrap(True)
        sl.addWidget(self.log_lbl)
        root.addWidget(side)

        # ---- cột chính ----
        main = QVBoxLayout()
        main.setContentsMargins(16, 16, 16, 8)
        main.setSpacing(12)
        root.addLayout(main, 1)

        bar = QFrame()
        bar.setObjectName("card")
        bl = QHBoxLayout(bar)
        bl.setContentsMargins(16, 12, 16, 12)
        bl.setSpacing(12)
        self.b_quick, self.b_deep = QPushButton("Quét nhanh"), QPushButton("Quét sâu")
        grp = QButtonGroup(self)
        for b in (self.b_quick, self.b_deep):
            b.setCheckable(True)
            b.setObjectName("seg")
            grp.addButton(b)
            bl.addWidget(b)
        self.b_quick.setChecked(True)
        bl.addSpacing(8)
        self.b_start = QPushButton("Bắt đầu quét")
        self.b_start.setObjectName("dark")
        self.b_start.clicked.connect(lambda: self.scan_requested.emit("deep" if self.b_deep.isChecked() else "quick"))
        self.b_stop = QPushButton("Dừng")
        self.b_stop.clicked.connect(self.cancel_requested)
        bl.addWidget(self.b_start)
        bl.addWidget(self.b_stop)
        pc = QVBoxLayout()
        pc.setSpacing(4)
        self.status = QLabel("Chọn thiết bị để bắt đầu")
        self.status.setObjectName("caption")
        self.prog = QProgressBar()
        self.prog.setTextVisible(False)
        self.prog.setFixedHeight(8)
        pc.addWidget(self.status)
        pc.addWidget(self.prog)
        bl.addLayout(pc, 1)
        main.addWidget(bar)

        fr = QHBoxLayout()
        self.search = QLineEdit()
        self.search.setPlaceholderText("Tìm theo tên…")
        self.search.setFixedWidth(240)
        self.search.setClearButtonEnabled(True)
        self.search.textChanged.connect(self._proxy.set_text)
        self.kind = QComboBox()
        self.kind.addItem("Tất cả loại", "")
        self.kind.currentIndexChanged.connect(lambda: self._proxy.set_kind(self.kind.currentData() or ""))
        self.b_all, self.b_none = QPushButton("Chọn tất cả"), QPushButton("Bỏ chọn")
        fr.addWidget(self.search)
        fr.addWidget(self.kind)
        fr.addStretch(1)
        fr.addWidget(self.b_all)
        fr.addWidget(self.b_none)
        main.addLayout(fr)

        self.table = QTableView()
        self.table.setModel(self._proxy)
        self.table.setSortingEnabled(True)
        self.table.setSelectionBehavior(QAbstractItemView.SelectionBehavior.SelectRows)
        self.table.setAlternatingRowColors(False)
        self.table.verticalHeader().hide()
        self.table.verticalHeader().setDefaultSectionSize(34)
        self.table.setShowGrid(False)
        h = self.table.horizontalHeader()
        h.setStretchLastSection(False)
        h.setSectionResizeMode(0, QHeaderView.ResizeMode.Stretch)
        for c in range(1, 6):
            h.setSectionResizeMode(c, QHeaderView.ResizeMode.ResizeToContents)
        main.addWidget(self.table, 1)
        self._refresh_buttons()

    # ---- API cho cửa sổ chính ----
    def set_devices(self, paths: list[str]) -> None:
        self.devices.clear()
        for p in paths:
            it = self.devices.addItem(p)
        for i, p in enumerate(paths):
            self.devices.item(i).setData(Qt.ItemDataRole.UserRole, p)

    def set_info(self, ev: dict) -> None:
        def v(k: str) -> str:
            return str(ev.get(k) or "không rõ")
        rem = {True: "có", False: "không"}.get(ev.get("removable"), "không rõ")
        vols = "\n".join(
            f"  #{i + 1}: {x['fs']} · {fmt_size(x['len'])}" for i, x in enumerate(ev.get("volumes", []))
        ) or "  (không nhận dạng được)"
        self.info.setText(
            f"{v('device')}\nHãng: {v('vendor')}\nModel: {v('model')}\nSerial: {v('serial')}\n"
            f"Giao tiếp: {v('bus')}\nRút được: {rem}\nDung lượng: {fmt_size(int(ev.get('size', 0)))}\n"
            f"Sector: {ev.get('sector', '?')} byte\nVolume:\n{vols}"
        )
        self.log_lbl.setText(f"Đã ghi {ev['log']}" if ev.get("log") and not ev.get("log_error") else
                             f"Không ghi được log: {ev.get('log_error')}" if ev.get("log_error") else "")

    def clear_info(self) -> None:
        self.info.setText("Chưa mở thiết bị")
        self.log_lbl.setText("")

    def add_kinds(self, kinds) -> None:
        for k in sorted(set(kinds) - self._kinds):
            self._kinds.add(k)
            self.kind.addItem(k, k)

    def reset_kinds(self) -> None:
        self._kinds.clear()
        self.kind.clear()
        self.kind.addItem("Tất cả loại", "")

    def set_status(self, text: str) -> None:
        self.status.setText(text)

    def set_progress(self, done: int, total: int) -> None:
        self.prog.setRange(0, 1000)
        self.prog.setValue(int(done * 1000 / total) if total else 0)

    def reset_progress(self) -> None:
        self.prog.setRange(0, 1000)
        self.prog.setValue(0)

    def set_session(self, open_: bool) -> None:
        self._session = open_
        self._refresh_buttons()

    def set_busy(self, busy: bool) -> None:
        self._busy = busy
        self._refresh_buttons()

    def _refresh_buttons(self) -> None:
        self.b_start.setEnabled(self._session and not self._busy)
        self.b_quick.setEnabled(not self._busy)
        self.b_deep.setEnabled(not self._busy)
        self.b_stop.setEnabled(self._busy)
        self.devices.setEnabled(not self._busy)
