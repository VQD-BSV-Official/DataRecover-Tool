"""Model bảng file tìm thấy + proxy lọc/sắp xếp."""
from __future__ import annotations

from PySide6.QtCore import QAbstractTableModel, QModelIndex, QSortFilterProxyModel, Qt, Signal
from PySide6.QtGui import QColor

COLUMNS = ["Tên", "Loại", "Kích thước", "Offset", "Nguồn", "Độ tin cậy"]
C_NAME, C_KIND, C_SIZE, C_OFF, C_SRC, C_CONF = range(6)


def fmt_size(n: int) -> str:
    v = float(n)
    for u in ("byte", "KB", "MB", "GB", "TB"):
        if v < 1024 or u == "TB":
            return f"{int(v)} {u}" if u == "byte" else f"{v:.1f} {u}"
        v /= 1024
    return f"{n}"


def conf_label(c: int) -> tuple[str, str]:
    if c >= 80:
        return f"{c} · Cao", "#4ADE80"
    if c >= 50:
        return f"{c} · Vừa", "#FBBF24"
    return f"{c} · Thấp", "#FB923C"


class FileModel(QAbstractTableModel):
    checked_changed = Signal()

    def __init__(self, parent=None):
        super().__init__(parent)
        self._rows: list[dict] = []
        self._checked: set[int] = set()

    # ---- dữ liệu ----
    def clear(self) -> None:
        self.beginResetModel()
        self._rows.clear()
        self._checked.clear()
        self.endResetModel()
        self.checked_changed.emit()

    def append(self, files: list[dict]) -> None:
        if not files:
            return
        n = len(self._rows)
        self.beginInsertRows(QModelIndex(), n, n + len(files) - 1)
        self._rows.extend(files)
        self.endInsertRows()

    def checked_ids(self) -> list[int]:
        return sorted(self._checked)

    def checked_size(self) -> int:
        return sum(r["size"] for r in self._rows if r["id"] in self._checked)

    def set_checked_rows(self, rows: list[int], flag: bool) -> None:
        for r in rows:
            fid = self._rows[r]["id"]
            (self._checked.add if flag else self._checked.discard)(fid)
        if self._rows:
            self.dataChanged.emit(self.index(0, 0), self.index(len(self._rows) - 1, 0), [Qt.ItemDataRole.CheckStateRole])
        self.checked_changed.emit()

    # ---- Qt ----
    def rowCount(self, parent=QModelIndex()) -> int:
        return 0 if parent.isValid() else len(self._rows)

    def columnCount(self, parent=QModelIndex()) -> int:
        return 0 if parent.isValid() else len(COLUMNS)

    def headerData(self, s, o, role=Qt.ItemDataRole.DisplayRole):
        if o == Qt.Orientation.Horizontal and role == Qt.ItemDataRole.DisplayRole:
            return COLUMNS[s]
        return None

    def flags(self, idx):
        f = Qt.ItemFlag.ItemIsEnabled | Qt.ItemFlag.ItemIsSelectable
        if idx.column() == C_NAME:
            f |= Qt.ItemFlag.ItemIsUserCheckable
        return f

    def data(self, idx, role=Qt.ItemDataRole.DisplayRole):
        if not idx.isValid():
            return None
        r, c = self._rows[idx.row()], idx.column()
        if role == Qt.ItemDataRole.CheckStateRole and c == C_NAME:
            return Qt.CheckState.Checked if r["id"] in self._checked else Qt.CheckState.Unchecked
        if role == Qt.ItemDataRole.UserRole:  # giá trị thô để sắp xếp
            return (r["path"] or r["name"], r["kind"], r["size"], r["offset"], r["source"], r["confidence"])[c]
        if role == Qt.ItemDataRole.ForegroundRole and c == C_CONF:
            return QColor(conf_label(r["confidence"])[1])
        if role == Qt.ItemDataRole.TextAlignmentRole and c in (C_SIZE, C_OFF):
            return int(Qt.AlignmentFlag.AlignRight | Qt.AlignmentFlag.AlignVCenter)
        if role == Qt.ItemDataRole.ToolTipRole and c == C_NAME:
            return r["path"] or r["name"]
        if role == Qt.ItemDataRole.DisplayRole:
            return (
                r["path"] or r["name"], r["kind"], fmt_size(r["size"]),
                f"0x{r['offset']:X}", r["source"], conf_label(r["confidence"])[0],
            )[c]
        return None

    def setData(self, idx, value, role=Qt.ItemDataRole.EditRole) -> bool:
        if role == Qt.ItemDataRole.CheckStateRole and idx.column() == C_NAME:
            on = getattr(value, "value", value) == 2
            fid = self._rows[idx.row()]["id"]
            (self._checked.add if on else self._checked.discard)(fid)
            self.dataChanged.emit(idx, idx, [role])
            self.checked_changed.emit()
            return True
        return False


class FileFilter(QSortFilterProxyModel):
    def __init__(self, parent=None):
        super().__init__(parent)
        self._text, self._kind = "", ""
        self.setSortRole(Qt.ItemDataRole.UserRole)

    def set_text(self, t: str) -> None:
        self._text = t.strip().lower()
        self.invalidateFilter()

    def set_kind(self, k: str) -> None:
        self._kind = k
        self.invalidateFilter()

    def filterAcceptsRow(self, row, parent) -> bool:
        m = self.sourceModel()
        if self._kind and m.data(m.index(row, C_KIND, parent), Qt.ItemDataRole.DisplayRole) != self._kind:
            return False
        if self._text:
            return self._text in str(m.data(m.index(row, C_NAME, parent), Qt.ItemDataRole.DisplayRole)).lower()
        return True

    def visible_source_rows(self) -> list[int]:
        return [self.mapToSource(self.index(i, 0)).row() for i in range(self.rowCount())]
