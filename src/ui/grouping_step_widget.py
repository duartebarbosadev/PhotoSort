from __future__ import annotations

import os
from typing import Dict, List, Optional

from PyQt6.QtCore import Qt, pyqtSignal
from PyQt6.QtGui import QIcon, QPixmap
from PyQt6.QtWidgets import (
    QAbstractItemView,
    QButtonGroup,
    QFrame,
    QHBoxLayout,
    QLabel,
    QProgressBar,
    QPushButton,
    QSplitter,
    QStackedWidget,
    QTreeWidget,
    QTreeWidgetItem,
    QVBoxLayout,
    QWidget,
)

from ui.advanced_image_viewer import ZoomableImageView


GROUPING_MODE_OPTIONS = [
    ("Current", "current"),
    ("Similarity", "similarity"),
    ("Face", "face"),
    ("Location", "location"),
    ("Mixed", "mixed"),
]


class GroupingStepWidget(QWidget):
    mode_changed = pyqtSignal(str)
    create_requested = pyqtSignal(str, dict)
    back_requested = pyqtSignal()
    skip_requested = pyqtSignal()
    select_folder_requested = pyqtSignal()

    def __init__(self, parent=None):
        super().__init__(parent)
        self._current_plan = None
        self._parent_window = parent
        self._create_widgets()
        self._create_layout()
        self._connect_signals()
        self.set_source_folder(None)

    # ── Widget construction ───────────────────────────────────────────

    def _create_widgets(self) -> None:
        # Top bar
        self.top_bar = QFrame()
        self.top_bar.setObjectName("groupingTopBar")

        self.back_button = QPushButton("← Back")
        self.back_button.setObjectName("groupingGhostButton")
        self.back_button.setVisible(False)

        self.folder_button = QPushButton("📁  Select Folder")
        self.folder_button.setObjectName("groupingFolderButton")

        self.folder_path_label = QLabel("No folder selected")
        self.folder_path_label.setObjectName("groupingFolderPath")

        # Mode pill toggle buttons
        self._mode_button_group = QButtonGroup(self)
        self._mode_button_group.setExclusive(True)
        self._mode_buttons: Dict[str, QPushButton] = {}
        for label, value in GROUPING_MODE_OPTIONS:
            btn = QPushButton(label)
            btn.setObjectName("groupingModePill")
            btn.setCheckable(True)
            self._mode_button_group.addButton(btn)
            self._mode_buttons[value] = btn
        self._mode_buttons["current"].setChecked(True)

        self.stats_label = QLabel()
        self.stats_label.setObjectName("groupingSummaryBadge")
        self.stats_label.setVisible(False)

        self.skip_button = QPushButton("Skip to Cull")
        self.skip_button.setObjectName("groupingGhostButton")
        self.skip_button.setEnabled(False)

        self.primary_button = QPushButton("Move files")
        self.primary_button.setObjectName("groupingPrimaryButton")
        self.primary_button.setMinimumHeight(34)
        self.primary_button.setEnabled(False)

        # Empty state page
        self.empty_state_frame = QFrame()
        self.empty_state_frame.setObjectName("groupingEmptyState")
        self._empty_icon = QLabel("🗂")
        self._empty_icon.setObjectName("groupingEmptyIcon")
        self._empty_icon.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self._empty_title = QLabel("Select a source folder to begin")
        self._empty_title.setObjectName("groupingEmptyTitle")
        self._empty_title.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self._empty_subtitle = QLabel(
            "PhotoSort will analyse your photos and generate a structured folder plan.\n"
            "Review the before/after trees, rename any group, then create."
        )
        self._empty_subtitle.setObjectName("groupingEmptySubtitle")
        self._empty_subtitle.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self._empty_subtitle.setWordWrap(True)
        self._empty_cta = QPushButton("📁  Select Folder")
        self._empty_cta.setObjectName("groupingEmptyCTA")

        # Before tree panel
        self.before_panel = QFrame()
        self.before_panel.setObjectName("groupingBeforePanel")
        self.before_header = QLabel("Before")
        self.before_header.setObjectName("groupingTreeHeader")
        self.before_desc = QLabel("Current structure")
        self.before_desc.setObjectName("groupingTreeDesc")
        self.before_tree = QTreeWidget()
        self.before_tree.setObjectName("groupingTree")
        self.before_tree.setColumnCount(1)
        self.before_tree.setHeaderHidden(True)
        self.before_tree.setRootIsDecorated(True)
        self.before_tree.setAlternatingRowColors(False)
        self.before_tree.setSelectionMode(QAbstractItemView.SelectionMode.SingleSelection)
        self.before_tree.setIconSize(self.before_tree.iconSize())

        # After tree panel
        self.after_panel = QFrame()
        self.after_panel.setObjectName("groupingAfterPanel")
        self.after_header = QLabel("After")
        self.after_header.setObjectName("groupingTreeHeader")
        self.after_desc = QLabel("Double-click a group name to rename it")
        self.after_desc.setObjectName("groupingTreeDesc")
        # preview_tree kept as attribute name for external API compatibility
        self.preview_tree = QTreeWidget()
        self.preview_tree.setObjectName("groupingTree")
        self.preview_tree.setColumnCount(1)
        self.preview_tree.setHeaderHidden(True)
        self.preview_tree.setRootIsDecorated(True)
        self.preview_tree.setAlternatingRowColors(False)
        self.preview_tree.setSelectionMode(QAbstractItemView.SelectionMode.SingleSelection)
        self.preview_tree.setIconSize(self.preview_tree.iconSize())

        # Preview panel (3rd column in splitter)
        self.preview_panel = QFrame()
        self.preview_panel.setObjectName("groupingPreviewPanel")
        self.preview_panel_header = QLabel("Preview")
        self.preview_panel_header.setObjectName("groupingTreeHeader")
        self.same_badge = QLabel("≡  No changes")
        self.same_badge.setObjectName("groupingSameBadge")
        self.same_badge.setVisible(False)
        # Stacked inside preview panel: 0 = hint, 1 = image
        self.preview_pane_stack = QStackedWidget()
        self.preview_hint_label = QLabel("Select a photo to preview")
        self.preview_hint_label.setObjectName("groupingPreviewHint")
        self.preview_hint_label.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self.large_preview_view = ZoomableImageView()
        self.large_preview_view.setObjectName("groupingLargePreview")
        self.large_preview_name = QLabel()
        self.large_preview_name.setObjectName("groupingSelectionName")
        self.large_preview_name.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self.large_preview_name.setWordWrap(True)

        # Stacked widget: 0 = empty state, 1 = split trees
        self.stacked = QStackedWidget()
        self.main_splitter = QSplitter(Qt.Orientation.Horizontal)
        self.main_splitter.setObjectName("groupingMainSplitter")
        self.main_splitter.setHandleWidth(1)

        # Bottom status bar
        self.bottom_bar = QFrame()
        self.bottom_bar.setObjectName("groupingBottomBar")

        self.loading_label = QLabel("Select a folder to start.")
        self.loading_label.setObjectName("groupingLoadingLabel")
        self.loading_bar = QProgressBar()
        self.loading_bar.setObjectName("groupingLoadingBar")
        self.loading_bar.setTextVisible(False)
        self.loading_bar.setRange(0, 1)
        self.loading_bar.setValue(0)
        self.loading_bar.setVisible(False)

        self.thumb_label = QLabel()
        self.thumb_label.setObjectName("groupingThumb")
        self.thumb_label.setFixedSize(52, 52)
        self.thumb_label.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self.thumb_label.setVisible(False)

        self.preview_selection_label = QLabel()
        self.preview_selection_label.setObjectName("groupingSelectionName")
        self.preview_selection_label.setVisible(False)

        self.preview_selection_meta = QLabel()
        self.preview_selection_meta.setObjectName("groupingPathValue")
        self.preview_selection_meta.setVisible(False)

        self.open_preview_button = QPushButton("Open Preview")
        self.open_preview_button.setObjectName("groupingSecondaryButton")
        self.open_preview_button.setEnabled(False)
        self.open_preview_button.setVisible(False)

        # Hidden compatibility shims used by external callers / tests
        self.output_root_label = QLabel()
        self.output_root_label.setVisible(False)
        self.preview_label = self.loading_label          # set_preview_text → loading_label
        self.preview_stats_label = self.stats_label      # alias
        self.preview_image_label = self.large_preview_view  # alias → ZoomableImageView

    def _create_layout(self) -> None:
        root = QVBoxLayout(self)
        root.setContentsMargins(0, 0, 0, 0)
        root.setSpacing(0)

        # ── Top bar ───────────────────────────────────────────────────
        tb = QHBoxLayout(self.top_bar)
        tb.setContentsMargins(16, 10, 16, 10)
        tb.setSpacing(6)
        tb.addWidget(self.back_button)
        tb.addWidget(self.folder_button)
        tb.addWidget(self.folder_path_label, 1)
        sep = QFrame()
        sep.setObjectName("groupingTopBarSep")
        sep.setFrameShape(QFrame.Shape.VLine)
        tb.addWidget(sep)
        for _, value in GROUPING_MODE_OPTIONS:
            tb.addWidget(self._mode_buttons[value])
        tb.addSpacing(10)
        tb.addWidget(self.stats_label)
        tb.addStretch(1)
        tb.addWidget(self.skip_button)
        tb.addSpacing(6)
        tb.addWidget(self.primary_button)

        root.addWidget(self.top_bar)
        self._add_hsep(root, "groupingBarSep")

        # ── Empty state layout ────────────────────────────────────────
        es = QVBoxLayout(self.empty_state_frame)
        es.setContentsMargins(0, 0, 0, 0)
        es.setSpacing(14)
        es.addStretch(1)
        es.addWidget(self._empty_icon)
        es.addWidget(self._empty_title)
        es.addWidget(self._empty_subtitle)
        cta_row = QHBoxLayout()
        cta_row.addStretch(1)
        cta_row.addWidget(self._empty_cta)
        cta_row.addStretch(1)
        es.addLayout(cta_row)
        es.addStretch(1)

        # ── Before panel ──────────────────────────────────────────────
        bl = QVBoxLayout(self.before_panel)
        bl.setContentsMargins(0, 0, 0, 0)
        bl.setSpacing(0)
        bh = QHBoxLayout()
        bh.setContentsMargins(16, 12, 16, 10)
        bh.setSpacing(8)
        bh.addWidget(self.before_header)
        bh.addWidget(self.before_desc)
        bh.addStretch(1)
        bl.addLayout(bh)
        self._add_hsep(bl, "groupingPanelSep")
        bl.addWidget(self.before_tree, 1)

        # ── After panel ───────────────────────────────────────────────
        al = QVBoxLayout(self.after_panel)
        al.setContentsMargins(0, 0, 0, 0)
        al.setSpacing(0)
        ah = QHBoxLayout()
        ah.setContentsMargins(16, 12, 16, 10)
        ah.setSpacing(8)
        ah.addWidget(self.after_header)
        ah.addWidget(self.after_desc)
        ah.addStretch(1)
        al.addLayout(ah)
        self._add_hsep(al, "groupingPanelSep")
        al.addWidget(self.preview_tree, 1)

        # ── Preview panel ─────────────────────────────────────────────
        pv = QVBoxLayout(self.preview_panel)
        pv.setContentsMargins(0, 0, 0, 0)
        pv.setSpacing(0)
        pvh = QHBoxLayout()
        pvh.setContentsMargins(16, 12, 16, 10)
        pvh.setSpacing(8)
        pvh.addWidget(self.preview_panel_header)
        pvh.addStretch(1)
        pvh.addWidget(self.same_badge)
        pv.addLayout(pvh)
        self._add_hsep(pv, "groupingPanelSep")
        # image page
        img_page = QWidget()
        img_layout = QVBoxLayout(img_page)
        img_layout.setContentsMargins(12, 12, 12, 10)
        img_layout.setSpacing(8)
        img_layout.addWidget(self.large_preview_view, 1)
        img_layout.addWidget(self.large_preview_name)
        self.preview_pane_stack.addWidget(self.preview_hint_label)  # index 0
        self.preview_pane_stack.addWidget(img_page)                 # index 1
        pv.addWidget(self.preview_pane_stack, 1)

        # ── Splitter assembly ─────────────────────────────────────────
        self.main_splitter.addWidget(self.before_panel)
        self.main_splitter.addWidget(self.after_panel)
        self.main_splitter.addWidget(self.preview_panel)
        self.main_splitter.setStretchFactor(0, 1)
        self.main_splitter.setStretchFactor(1, 1)
        self.main_splitter.setStretchFactor(2, 1)
        self.main_splitter.setSizes([380, 380, 320])

        # ── Stacked pages ─────────────────────────────────────────────
        self.stacked.addWidget(self.empty_state_frame)   # index 0
        self.stacked.addWidget(self.main_splitter)       # index 1

        root.addWidget(self.stacked, 1)
        self._add_hsep(root, "groupingBarSep")

        # ── Bottom status bar ─────────────────────────────────────────
        bb = QHBoxLayout(self.bottom_bar)
        bb.setContentsMargins(16, 8, 16, 8)
        bb.setSpacing(10)
        info_col = QVBoxLayout()
        info_col.setSpacing(2)
        info_col.addWidget(self.preview_selection_label)
        info_col.addWidget(self.preview_selection_meta)
        bb.addLayout(info_col)
        bb.addStretch(1)
        loading_col = QVBoxLayout()
        loading_col.setSpacing(4)
        loading_col.addWidget(self.loading_label)
        loading_col.addWidget(self.loading_bar)
        bb.addLayout(loading_col)

        root.addWidget(self.bottom_bar)

    @staticmethod
    def _add_hsep(layout, name: str) -> None:
        sep = QFrame()
        sep.setObjectName(name)
        sep.setFrameShape(QFrame.Shape.HLine)
        layout.addWidget(sep)

    # ── Signal wiring ─────────────────────────────────────────────────

    def _connect_signals(self) -> None:
        self._mode_button_group.buttonClicked.connect(self._emit_mode_changed)
        self.primary_button.clicked.connect(self._emit_create_requested)
        self.back_button.clicked.connect(self.back_requested.emit)
        self.skip_button.clicked.connect(self.skip_requested.emit)
        self.folder_button.clicked.connect(self.select_folder_requested.emit)
        self._empty_cta.clicked.connect(self.select_folder_requested.emit)
        self.preview_tree.currentItemChanged.connect(self._handle_after_item_changed)
        self.before_tree.currentItemChanged.connect(self._handle_before_item_changed)
        self.preview_tree.itemDoubleClicked.connect(self._handle_tree_double_click)

    def _emit_mode_changed(self) -> None:
        self.mode_changed.emit(self.current_mode())

    def _emit_create_requested(self) -> None:
        self.create_requested.emit(self.current_mode(), self.get_group_name_overrides())

    # ── Public API ────────────────────────────────────────────────────

    def current_mode(self) -> str:
        for value, btn in self._mode_buttons.items():
            if btn.isChecked():
                return value
        return "current"

    def set_current_mode(self, mode: str) -> None:
        btn = self._mode_buttons.get(mode)
        if btn:
            btn.setChecked(True)

    def set_preview_text(self, text: str) -> None:
        self.loading_label.setText(text or "")

    def set_output_root_text(self, text: str) -> None:
        # Displayed in after-panel header desc; kept for API compatibility
        if text:
            self.after_desc.setText(text)

    def set_busy(self, busy: bool) -> None:
        self.primary_button.setEnabled(not busy and self._current_plan is not None)
        for btn in self._mode_buttons.values():
            btn.setEnabled(not busy and self.has_source_folder())
        self.skip_button.setEnabled(not busy and self.has_source_folder())
        self.folder_button.setEnabled(not busy)
        self.back_button.setEnabled(not busy)
        self.primary_button.setText(
            "Grouping…" if busy else "Move files"
        )

    def set_back_visible(self, visible: bool) -> None:
        self.back_button.setVisible(visible)

    def set_source_folder(self, folder_path: Optional[str]) -> None:
        has_folder = bool(folder_path)
        if has_folder:
            parts = (folder_path or "").replace("\\", "/").split("/")
            display = ("…/" + "/".join(parts[-3:])) if len(parts) > 4 else folder_path
            self.folder_path_label.setText(display)
            self.folder_path_label.setToolTip(folder_path)
            self.folder_button.setText("📁  Change")
            self.stacked.setCurrentIndex(1)
        else:
            self.folder_path_label.setText("No folder selected")
            self.folder_button.setText("📁  Select Folder")
            self.stacked.setCurrentIndex(0)

        for btn in self._mode_buttons.values():
            btn.setEnabled(has_folder)
        self.skip_button.setEnabled(has_folder)
        self.primary_button.setEnabled(has_folder and self._current_plan is not None)
        if not has_folder:
            self.before_tree.clear()
            self.preview_tree.clear()
            self._current_plan = None
            self.stats_label.setVisible(False)
            self.loading_label.setText("Select a folder to start.")
            self.loading_bar.setVisible(False)
            self._clear_selected_preview()

    def has_source_folder(self) -> bool:
        return self.folder_path_label.text() != "No folder selected"

    def set_preview_plan(self, plan, output_root: Optional[str] = None) -> None:
        self._current_plan = plan
        count_label = (
            f"{len(plan.groups)} folders  ·  "
            f"{len(plan.unassigned_paths)} unassigned  ·  "
            f"{len(plan.skipped_paths)} skipped"
        )
        self.stats_label.setText(count_label)
        self.stats_label.setVisible(True)
        self.loading_label.setText("Preview ready")
        self.loading_bar.setVisible(False)

        # ── After tree ────────────────────────────────────────────────
        self.preview_tree.clear()
        root_display = output_root or plan.output_root or "Selected folder"
        root_name = os.path.basename(os.path.normpath(root_display)) or root_display
        root_item = QTreeWidgetItem([f"📁  {root_name}"])
        root_item.setFlags(root_item.flags() & ~Qt.ItemFlag.ItemIsEditable)
        self.preview_tree.addTopLevelItem(root_item)
        current_parent = root_item

        for group in plan.groups:
            count = len(group.source_paths)
            folder_item = QTreeWidgetItem([f"📁  {group.group_label}"])
            folder_item.setData(0, Qt.ItemDataRole.UserRole, group.group_id)
            folder_item.setFlags(folder_item.flags() | Qt.ItemFlag.ItemIsEditable)
            folder_item.setData(0, Qt.ItemDataRole.EditRole, group.group_label)
            folder_item.setToolTip(0, f"Double-click to rename · {count} photo(s)")
            current_parent.addChild(folder_item)
            for source_path in group.source_paths[:12]:
                fi = QTreeWidgetItem([os.path.basename(source_path)])
                fi.setFlags(fi.flags() & ~Qt.ItemFlag.ItemIsEditable)
                fi.setData(0, Qt.ItemDataRole.UserRole + 1, source_path)
                self._set_preview_icon(fi, source_path)
                folder_item.addChild(fi)
            remaining = len(group.source_paths) - min(12, len(group.source_paths))
            if remaining > 0:
                more = QTreeWidgetItem([f"…  {remaining} more"])
                more.setFlags(more.flags() & ~Qt.ItemFlag.ItemIsEditable)
                folder_item.addChild(more)

        if plan.unassigned_paths:
            ua = QTreeWidgetItem([f"📁  Unassigned  ({len(plan.unassigned_paths)})"])
            ua.setFlags(ua.flags() & ~Qt.ItemFlag.ItemIsEditable)
            current_parent.addChild(ua)
            for p in plan.unassigned_paths[:12]:
                fi = QTreeWidgetItem([os.path.basename(p)])
                fi.setFlags(fi.flags() & ~Qt.ItemFlag.ItemIsEditable)
                fi.setData(0, Qt.ItemDataRole.UserRole + 1, p)
                self._set_preview_icon(fi, p)
                ua.addChild(fi)

        if plan.skipped_paths:
            sk = QTreeWidgetItem([f"📁  Skipped  ({len(plan.skipped_paths)})"])
            sk.setFlags(sk.flags() & ~Qt.ItemFlag.ItemIsEditable)
            current_parent.addChild(sk)
            for p in plan.skipped_paths[:12]:
                fi = QTreeWidgetItem([os.path.basename(p)])
                fi.setFlags(fi.flags() & ~Qt.ItemFlag.ItemIsEditable)
                fi.setData(0, Qt.ItemDataRole.UserRole + 1, p)
                sk.addChild(fi)

        self.preview_tree.expandAll()
        self.primary_button.setEnabled(True)
        self.preview_tree.setCurrentItem(None)

        # ── Before tree ───────────────────────────────────────────────
        self._build_before_tree(plan)
        self._clear_selected_preview()
        # ── Equality badge ────────────────────────────────────
        self.same_badge.setVisible(self._plan_has_no_effective_changes(plan))
    def _build_before_tree(self, plan) -> None:
        self.before_tree.clear()
        all_paths: List[str] = []
        for group in plan.groups:
            all_paths.extend(group.source_paths)
        all_paths.extend(plan.unassigned_paths)
        all_paths.extend(plan.skipped_paths)
        if not all_paths:
            return

        common = (
            os.path.commonpath(all_paths)
            if len(all_paths) > 1
            else os.path.dirname(all_paths[0])
        )
        root_name = os.path.basename(common) or common
        root_item = QTreeWidgetItem([f"📁  {root_name}"])
        root_item.setFlags(root_item.flags() & ~Qt.ItemFlag.ItemIsEditable)
        self.before_tree.addTopLevelItem(root_item)

        dir_items: Dict[str, QTreeWidgetItem] = {common: root_item}

        def ensure_dir(path: str) -> QTreeWidgetItem:
            if path in dir_items:
                return dir_items[path]
            parent_item = ensure_dir(os.path.dirname(path))
            item = QTreeWidgetItem([f"📁  {os.path.basename(path)}"])
            item.setFlags(item.flags() & ~Qt.ItemFlag.ItemIsEditable)
            parent_item.addChild(item)
            dir_items[path] = item
            return item

        for p in sorted(all_paths):
            d = os.path.dirname(p)
            parent_item = ensure_dir(d) if d != common else root_item
            fi = QTreeWidgetItem([os.path.basename(p)])
            fi.setFlags(fi.flags() & ~Qt.ItemFlag.ItemIsEditable)
            fi.setData(0, Qt.ItemDataRole.UserRole + 1, p)
            self._set_preview_icon(fi, p)
            parent_item.addChild(fi)

        self.before_tree.expandAll()

    def get_group_name_overrides(self) -> Dict[str, str]:
        overrides: Dict[str, str] = {}
        root = self.preview_tree.topLevelItem(0)
        if root is not None:
            self._collect_overrides(root, overrides)
        return overrides

    def _collect_overrides(self, item: QTreeWidgetItem, overrides: Dict[str, str]) -> None:
        group_id = item.data(0, Qt.ItemDataRole.UserRole)
        if group_id:
            text = item.text(0).strip()
            for prefix in ("📁  ", "📁 ", "📁"):
                if text.startswith(prefix):
                    text = text[len(prefix):].strip()
                    break
            overrides[str(group_id)] = text or item.text(0)
        for i in range(item.childCount()):
            self._collect_overrides(item.child(i), overrides)

    def set_loading_state(
        self, message: str, busy: bool, progress: Optional[int] = None
    ) -> None:
        self.loading_label.setText(message)
        self.loading_bar.setVisible(busy)
        if not busy:
            self.loading_bar.setRange(0, 1)
            self.loading_bar.setValue(0)
            return
        if progress is None or progress < 0:
            self.loading_bar.setRange(0, 0)
        else:
            self.loading_bar.setRange(0, 100)
            self.loading_bar.setValue(max(0, min(100, progress)))

    # ── Private helpers ───────────────────────────────────────────────

    def _set_preview_icon(self, item: QTreeWidgetItem, source_path: str) -> None:
        image_pipeline = getattr(self._parent_window, "image_pipeline", None)
        if image_pipeline is None:
            return
        try:
            pixmap = image_pipeline.get_thumbnail_qpixmap(source_path)
            if pixmap is not None:
                item.setIcon(0, QIcon(pixmap))
        except Exception:
            return

    def _handle_after_item_changed(
        self, current: Optional[QTreeWidgetItem], _prev: Optional[QTreeWidgetItem]
    ) -> None:
        if current is None:
            return
        source_path = current.data(0, Qt.ItemDataRole.UserRole + 1)
        if source_path:
            self._update_selected_preview(str(source_path))
        else:
            self._clear_selected_preview()

    def _handle_before_item_changed(
        self, current: Optional[QTreeWidgetItem], _prev: Optional[QTreeWidgetItem]
    ) -> None:
        if current is None:
            return
        source_path = current.data(0, Qt.ItemDataRole.UserRole + 1)
        if source_path:
            self._update_selected_preview(str(source_path))

    def _handle_tree_double_click(self, item: QTreeWidgetItem, column: int) -> None:
        if item.flags() & Qt.ItemFlag.ItemIsEditable:
            self.preview_tree.editItem(item, column)

    def _clear_selected_preview(self) -> None:
        self.large_preview_view.clear()
        self.large_preview_name.clear()
        self.preview_pane_stack.setCurrentIndex(0)
        self.preview_selection_label.setVisible(False)
        self.preview_selection_meta.setVisible(False)
        # compat shim
        self.thumb_label.clear()
        self.thumb_label.setVisible(False)

    def _update_selected_preview(self, source_path: str) -> None:
        image_pipeline = getattr(self._parent_window, "image_pipeline", None)
        pixmap: Optional[QPixmap] = None
        if image_pipeline:
            pixmap = image_pipeline.get_preview_qpixmap(
                source_path, display_max_size=(8000, 8000)
            )
            if pixmap is None or pixmap.isNull():
                pixmap = image_pipeline.get_thumbnail_qpixmap(source_path)
        if pixmap and not pixmap.isNull():
            self.large_preview_view.set_image(pixmap)
        else:
            self.large_preview_view.clear()
        self.large_preview_name.setText(os.path.basename(source_path))
        self.preview_pane_stack.setCurrentIndex(1)
        # bottom bar info
        self.preview_selection_label.setText(os.path.basename(source_path))
        self.preview_selection_label.setVisible(True)
        self.preview_selection_meta.setText(source_path)
        self.preview_selection_meta.setVisible(True)

    def _plan_has_no_effective_changes(self, plan) -> bool:
        """Return True when every file is already in a folder matching its target group."""
        if not plan.groups:
            return False
        for group in plan.groups:
            for path in group.source_paths:
                if os.path.basename(os.path.dirname(path)) != group.group_label:
                    return False
        return True
