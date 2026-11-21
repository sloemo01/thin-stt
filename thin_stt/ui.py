"""Thin STT main window."""

import os
import subprocess
import sys
import tempfile
from pathlib import Path

from PySide6.QtCore import Qt, QThread, Signal
from PySide6.QtGui import QAction, QKeySequence
from PySide6.QtWidgets import (
    QApplication, QComboBox, QFrame, QGridLayout, QHBoxLayout, QLabel,
    QMainWindow, QMenu, QMessageBox, QProgressBar, QPushButton, QKeySequenceEdit,
    QScrollArea, QStyle, QSystemTrayIcon, QTabWidget, QVBoxLayout, QWidget,
)

from . import audio, config, engines, hotkeys, models, typer

APP_ROOT = Path(__file__).resolve().parent.parent

STYLESHEET = """
QWidget { background: #09090b; color: #e4e4e7; font-family: 'Segoe UI', 'Inter'; font-size: 13px; }
QMainWindow, QDialog { background: #09090b; }
QTabWidget::pane { border: none; }
QTabBar::tab { background: transparent; color: #a1a1aa; padding: 6px 16px;
               border: 1px solid transparent; border-radius: 6px; margin: 4px 2px 0 2px; }
QTabBar::tab:selected { color: #fafafa; background: #18181b; border-color: #27272a; }
QTabBar::tab:hover:!selected { color: #fafafa; }

QLabel#muted { color: #a1a1aa; }
QLabel#section { color: #fafafa; font-weight: 700; font-size: 13px; padding-top: 10px; }
QLabel#engine-badge { background: #27272a; color: #a1a1aa; border-radius: 4px;
                      padding: 1px 7px; font-size: 11px; }
QLabel#status { color: #a1a1aa; }

QWidget#card { background: #18181b; border: 1px solid #27272a; border-radius: 8px; }

QPushButton { background: #fafafa; color: #09090b; border: none; border-radius: 6px;
              padding: 5px 14px; font-weight: 600; }
QPushButton:hover:!disabled { background: #d4d4d8; }
QPushButton:disabled { background: #27272a; color: #52525b; }
QPushButton#secondary { background: transparent; color: #e4e4e7; border: 1px solid #3f3f46; }
QPushButton#secondary:hover:!disabled { background: #27272a; }
QPushButton#destructive { background: transparent; color: #f87171; border: 1px solid #3f3f46; }
QPushButton#destructive:hover:!disabled { background: #450a0a; border-color: #7f1d1d; }
QPushButton#danger { background: #7f1d1d; color: #fecaca; font-weight: 700; padding: 10px; }
QPushButton#danger:hover { background: #991b1b; }
QPushButton#ghost { background: transparent; color: #f87171; border: none; font-size: 14px; }
QPushButton#ghost:hover:!disabled { background: #27272a; border-radius: 6px; }
QPushButton#ghost:disabled { color: #3f3f46; }

QKeySequenceEdit, QComboBox { background: #18181b; border: 1px solid #27272a;
                              border-radius: 6px; padding: 4px 8px; color: #e4e4e7; }
QKeySequenceEdit:focus, QComboBox:focus { border-color: #a1a1aa; }
QComboBox QAbstractItemView { background: #18181b; color: #e4e4e7; border: 1px solid #27272a;
                              selection-background-color: #27272a; outline: none; }
QComboBox::drop-down { border: none; width: 20px; }

QProgressBar { background: #27272a; border: none; border-radius: 5px;
               text-align: center; color: #09090b; font-size: 10px; }
QProgressBar::chunk { background: #fafafa; border-radius: 5px; }

QScrollArea { border: none; background: transparent; }
QStatusBar { background: #09090b; color: #a1a1aa; border-top: 1px solid #27272a; }
QStatusBar::item { border: none; }
QFrame[frameShape="4"], QFrame[frameShape="5"] { background: #27272a; max-height: 1px; border: none; }
QToolTip { background: #18181b; color: #e4e4e7; border: 1px solid #27272a; padding: 4px; }
QMenu { background: #18181b; border: 1px solid #27272a; }
QMenu::item:selected { background: #27272a; }
"""


class Worker(QThread):
    done = Signal(object)
    failed = Signal(str)
    progress = Signal(int, int)

    def __init__(self, fn, parent=None):
        super().__init__(parent)
        self._fn = fn

    def run(self):
        try:
            self.done.emit(self._fn(lambda d, t: self.progress.emit(d, t)))
        except Exception as e:  # noqa: BLE001
            self.failed.emit(str(e))


class RecordingOverlay(QWidget):
    """Small always-on-top pill shown while recording."""

    _CSS = ("background: rgba(24,24,27,240); color: {c}; font-weight: bold;"
            "padding: 8px 16px; border-radius: 14px; font-size: 12pt;"
            "border: 1px solid #3f3f46;")

    def __init__(self):
        super().__init__(None, Qt.FramelessWindowHint | Qt.WindowStaysOnTopHint
                         | Qt.Tool | Qt.WindowTransparentForInput)
        self.setAttribute(Qt.WA_TranslucentBackground)
        lay = QHBoxLayout(self)
        lay.setContentsMargins(0, 0, 0, 0)
        self._label = QLabel()
        lay.addWidget(self._label)

    def _place(self):
        self.adjustSize()
        screen = QApplication.primaryScreen().availableGeometry()
        self.move(screen.center().x() - self.width() // 2, screen.top() + 24)
        self.show()

    def show_recording(self, model_label: str):
        self._label.setStyleSheet(self._CSS.format(c="#ff5555"))
        self._label.setText(f"● Recording — {model_label}  (press hotkey again)")
        self._place()

    def show_transcribing(self):
        self._label.setStyleSheet(self._CSS.format(c="#ffd75f"))
        self._label.setText("… Transcribing …")
        self._place()


class ModelRow(QWidget):
    download_clicked = Signal(str)
    delete_clicked = Signal(str)
    record_clicked = Signal(str)
    hotkey_changed = Signal(str, str)

    def __init__(self, model: models.Model, cfg: dict):
        super().__init__()
        self.setObjectName("card")
        self.model = model
        g = QGridLayout(self)
        g.setContentsMargins(12, 8, 12, 8)
        g.setHorizontalSpacing(10)

        name = QLabel(model.label)
        name.setStyleSheet("font-weight:600; color:#fafafa; background:transparent;")
        name.setMinimumWidth(185)
        engine = QLabel(model.engine)
        engine.setObjectName("engine-badge")
        engine.setStyleSheet("background:#27272a;")
        self.size_lbl = QLabel(f"~{model.approx_mb} MB")
        self.status_lbl = QLabel("—")
        self.status_lbl.setObjectName("status")
        self.status_lbl.setStyleSheet("background:transparent;")
        self.dl_btn = QPushButton("Download")
        self.del_btn = QPushButton("Delete")
        self.del_btn.setObjectName("destructive")
        self.rec_btn = QPushButton("●")
        self.rec_btn.setObjectName("ghost")
        self.rec_btn.setFixedWidth(34)
        self.rec_btn.setToolTip("Record now with this model")
        self.hk_edit = QKeySequenceEdit()
        self.hk_edit.setMaximumWidth(140)
        seq = cfg["hotkeys"].get(model.id, "")
        if seq:
            qt = (seq.replace("ctrl", "Ctrl").replace("alt", "Alt")
                     .replace("shift", "Shift").replace("win", "Meta")
                     .replace("esc", "Esc"))
            self.hk_edit.setKeySequence(QKeySequence.fromString(qt, QKeySequence.PortableText))

        g.addWidget(name, 0, 0)
        g.addWidget(engine, 0, 1)
        g.addWidget(self.size_lbl, 0, 2)
        g.addWidget(self.status_lbl, 0, 3)
        g.addWidget(self.rec_btn, 0, 4)
        g.addWidget(self.hk_edit, 0, 5)
        g.addWidget(self.dl_btn, 0, 6)
        g.addWidget(self.del_btn, 0, 7)

        self.progress = QProgressBar()
        self.progress.setMaximumHeight(10)
        self.progress.setTextVisible(True)
        self.progress.hide()
        g.addWidget(self.progress, 1, 0, 1, 8)

        self.dl_btn.clicked.connect(lambda: self.download_clicked.emit(model.id))
        self.del_btn.clicked.connect(lambda: self.delete_clicked.emit(model.id))
        self.rec_btn.clicked.connect(lambda: self.record_clicked.emit(model.id))
        self.hk_edit.editingFinished.connect(self._hk_done)

    def _hk_done(self):
        ks = self.hk_edit.keySequence()
        if ks.count() > 1:  # keep only the first chord (Enter etc. isn't part of it)
            ks = QKeySequence(ks[0])
            self.hk_edit.setKeySequence(ks)
        seq = ks.toString(QKeySequence.PortableText)
        self.hotkey_changed.emit(self.model.id, hotkeys.qt_seq_to_hotkey(seq))

    def set_status(self, text: str, color: str = "#a1a1aa"):
        self.status_lbl.setText(text)
        self.status_lbl.setStyleSheet(f"color:{color}; background:transparent;")


class MainWindow(QMainWindow):
    hotkey_triggered = Signal(str)

    def __init__(self):
        super().__init__()
        self.setWindowTitle("Thin STT")
        self.resize(820, 480)
        self.cfg = config.load()
        self.recorder = None
        self.recording_model = None
        self.transcriber = engines.Transcriber()
        self.hk = hotkeys.HotkeyManager(self._hk_cb)
        self.overlay = RecordingOverlay()
        self.rows: dict[str, ModelRow] = {}
        self._workers: list[Worker] = []

        tabs = QTabWidget()
        tabs.addTab(self._build_models_tab(), "Models")
        tabs.addTab(self._build_settings_tab(), "Settings")
        self.setCentralWidget(tabs)

        self.statusBar().showMessage("Ready")
        self.hotkey_triggered.connect(self.toggle_recording)
        self._apply_hotkeys()
        self._refresh_rows()
        self._setup_tray()
        self._fetch_sizes()

    def _run_worker(self, fn, done=None, failed=None, progress=None):
        w = Worker(fn, self)
        self._workers.append(w)
        w.finished.connect(lambda: self._workers.remove(w) if w in self._workers else None)
        if done:
            w.done.connect(done)
        if failed:
            w.failed.connect(failed)
        if progress:
            w.progress.connect(progress)
        w.start()
        return w

    # ---------- Models tab ----------
    def _build_models_tab(self) -> QWidget:
        w = QWidget()
        v = QVBoxLayout(w)
        hint = QLabel("Hotkey works from any app: press → speak → press again → text is typed.")
        hint.setObjectName("muted")
        v.addWidget(hint)
        scroll = QScrollArea()
        scroll.setWidgetResizable(True)
        inner = QWidget()
        grid = QVBoxLayout(inner)
        grid.setContentsMargins(4, 4, 4, 4)
        grid.setSpacing(2)
        last_engine = None
        for m in models.MODELS:
            if m.engine != last_engine:
                last_engine = m.engine
                hdr = QLabel("OpenAI Whisper" if m.engine == "whisper" else "NVIDIA Parakeet")
                hdr.setObjectName("section")
                grid.addWidget(hdr)
            row = ModelRow(m, self.cfg)
            row.download_clicked.connect(self.start_download)
            row.delete_clicked.connect(self.delete_model)
            row.record_clicked.connect(self.toggle_recording)
            row.hotkey_changed.connect(self._row_hotkey_changed)
            self.rows[m.id] = row
            grid.addWidget(row)
        grid.addStretch(1)
        scroll.setWidget(inner)
        v.addWidget(scroll)
        return w

    def _refresh_rows(self):
        for m in models.MODELS:
            row = self.rows[m.id]
            if models.is_downloaded(m):
                mb = models.disk_size_mb(m)
                row.size_lbl.setText(f"{mb} MB" if mb else f"~{m.approx_mb} MB")
                row.set_status("ready", "#4caf50")
                row.del_btn.setEnabled(True)
                row.dl_btn.setEnabled(False)
                row.rec_btn.setEnabled(True)
            else:
                row.set_status("not downloaded")
                row.del_btn.setEnabled(False)
                row.dl_btn.setEnabled(True)
                row.rec_btn.setEnabled(False)

    def _fetch_sizes(self):
        def work(progress):
            out = {}
            for m in models.MODELS:
                if not models.is_downloaded(m):
                    try:
                        s = models.remote_size_bytes(m)
                        if s:
                            out[m.id] = s
                    except Exception:
                        pass
            return out

        self._run_worker(work, done=self._sizes_fetched)

    def _sizes_fetched(self, sizes: dict):
        self.cfg["remote_sizes"].update(sizes)
        config.save(self.cfg)
        for mid, b in sizes.items():
            if mid in self.rows and not models.is_downloaded(models.BY_ID[mid]):
                self.rows[mid].size_lbl.setText(f"~{b // (1024*1024)} MB")

    # ---------- download / delete ----------
    def start_download(self, model_id: str):
        m = models.BY_ID[model_id]
        row = self.rows[model_id]
        row.dl_btn.setEnabled(False)
        row.progress.show()
        row.progress.setRange(0, 0)
        row.set_status("downloading", "#2196f3")
        self._run_worker(
            lambda p: models.download(m, p),
            done=lambda _: self._dl_done(model_id),
            failed=lambda msg: self._dl_failed(row, msg),
            progress=lambda d, t: self._dl_progress(row, d, t),
        )

    def _dl_progress(self, row: ModelRow, done: int, total: int):
        if total > 0:
            row.progress.setRange(0, 100)
            row.progress.setValue(int(done * 100 / total))
            row.progress.setFormat(f"{done//(1024*1024)} MB / {total//(1024*1024)} MB")

    def _dl_done(self, model_id: str):
        row = self.rows[model_id]
        row.progress.hide()
        self._refresh_rows()
        self.statusBar().showMessage(f"Downloaded {models.BY_ID[model_id].label}", 5000)

    def _dl_failed(self, row: ModelRow, msg: str):
        row.progress.hide()
        row.dl_btn.setEnabled(True)
        row.set_status("download failed", "#f44336")
        self.statusBar().showMessage(f"Download failed: {msg}", 8000)

    def delete_model(self, model_id: str):
        m = models.BY_ID[model_id]
        if QMessageBox.question(self, "Delete model",
                                f"Delete {m.label} from disk?") != QMessageBox.Yes:
            return
        self.transcriber.drop(model_id)
        models.delete(m)
        self._refresh_rows()
        self.statusBar().showMessage(f"Deleted {m.label}", 5000)

    # ---------- hotkeys ----------
    def _hk_cb(self, model_id: str):
        self.hotkey_triggered.emit(model_id)

    def _apply_hotkeys(self):
        self.hk.clear_all()
        for mid, hk in self.cfg["hotkeys"].items():
            if hk:
                self.hk.set(mid, hk)

    def _row_hotkey_changed(self, model_id: str, hk: str):
        # editingFinished fires on focus-out too; ignore empty/no-change writes
        # so a saved hotkey isn't silently cleared.
        if not hk or hk == self.cfg["hotkeys"].get(model_id, ""):
            return
        self.cfg["hotkeys"][model_id] = hk
        config.save(self.cfg)
        self.hk.set(model_id, hk)
        self.statusBar().showMessage(
            f"{models.BY_ID[model_id].label} hotkey: {hk}", 4000)

    # ---------- record / transcribe ----------
    def toggle_recording(self, model_id: str):
        m = models.BY_ID[model_id]
        if self.recorder is not None and self.recorder.recording:
            self._stop_and_transcribe()
            return
        if not models.is_downloaded(m):
            self.statusBar().showMessage(f"{m.label} is not downloaded yet", 5000)
            return
        try:
            self.recorder = audio.Recorder(device=self.cfg["mic_device"])
            self.recorder.start()
        except Exception as e:  # noqa: BLE001
            self.statusBar().showMessage(f"Mic error: {e}", 8000)
            self.recorder = None
            return
        self.recording_model = model_id
        self.overlay.show_recording(m.label)

    def _stop_and_transcribe(self):
        model_id = self.recording_model
        try:
            wav = self.recorder.stop(config.recordings_dir() / "last.wav")
        except Exception as e:  # noqa: BLE001
            if self.recorder:
                self.recorder.abort()
            self.overlay.hide()
            self.statusBar().showMessage(f"Recording failed: {e}", 8000)
            return
        self.recording_model = None
        self.overlay.show_transcribing()
        m = models.BY_ID[model_id]
        self._run_worker(
            lambda p: self.transcriber.transcribe(model_id, m.load_name, wav),
            done=self._transcribed,
            failed=self._transcribe_failed,
        )

    def _transcribed(self, text: str):
        self.overlay.hide()
        typer.insert_text(text, self.cfg["paste_mode"], QApplication.clipboard())
        self.statusBar().showMessage(f"{self.cfg['paste_mode']}: {text[:80]}", 8000)

    def _transcribe_failed(self, msg: str):
        self.overlay.hide()
        self.statusBar().showMessage(f"Transcription failed: {msg}", 8000)

    # ---------- Settings tab ----------
    def _build_settings_tab(self) -> QWidget:
        w = QWidget()
        v = QVBoxLayout(w)
        g = QGridLayout()
        v.addLayout(g)

        g.addWidget(QLabel("Microphone"), 0, 0)
        self.mic_combo = QComboBox()
        self.mic_combo.addItem("System default", None)
        for idx, name in audio.input_devices():
            self.mic_combo.addItem(name, idx)
            if self.cfg["mic_device"] == idx:
                self.mic_combo.setCurrentIndex(self.mic_combo.count() - 1)
        self.mic_combo.currentIndexChanged.connect(self._mic_changed)
        g.addWidget(self.mic_combo, 0, 1)

        g.addWidget(QLabel("Insert text via"), 1, 0)
        self.paste_combo = QComboBox()
        for label, val in [("Paste (Ctrl+V)", "paste"),
                           ("Type keystrokes", "type"),
                           ("Clipboard only", "clipboard")]:
            self.paste_combo.addItem(label, val)
            if self.cfg["paste_mode"] == val:
                self.paste_combo.setCurrentIndex(self.paste_combo.count() - 1)
        self.paste_combo.currentIndexChanged.connect(self._paste_changed)
        g.addWidget(self.paste_combo, 1, 1)

        g.addWidget(QLabel("Model cache"), 2, 0)
        cache_lbl = QLabel(str(models._HUB_CACHE))
        cache_lbl.setTextInteractionFlags(Qt.TextSelectableByMouse)
        g.addWidget(cache_lbl, 2, 1)
        open_btn = QPushButton("Open models folder")
        open_btn.clicked.connect(lambda: os.startfile(models._HUB_CACHE))  # noqa: S606
        g.addWidget(open_btn, 3, 1)

        delall = QPushButton("Delete ALL downloaded models")
        delall.clicked.connect(self._delete_all_models)
        g.addWidget(delall, 4, 1)

        line = QFrame()
        line.setFrameShape(QFrame.HLine)
        v.addWidget(line)

        danger = QPushButton("Completely delete this app")
        danger.setObjectName("danger")
        danger.clicked.connect(self._uninstall)
        v.addWidget(danger)
        note = QLabel("Removes all models, settings, and the app folder itself.")
        note.setObjectName("muted")
        v.addWidget(note)
        v.addStretch(1)
        return w

    def _mic_changed(self, i):
        self.cfg["mic_device"] = self.mic_combo.itemData(i)
        config.save(self.cfg)

    def _paste_changed(self, i):
        self.cfg["paste_mode"] = self.paste_combo.itemData(i)
        config.save(self.cfg)

    def _delete_all_models(self):
        if QMessageBox.question(self, "Delete all models",
                                "Delete every downloaded model?") != QMessageBox.Yes:
            return
        self.transcriber.drop_all()
        for m in models.MODELS:
            models.delete(m)
        self._refresh_rows()

    def _uninstall(self):
        r = QMessageBox.warning(
            self, "Completely delete app",
            "This deletes all downloaded models, settings, and the app folder.\n"
            "Continue?", QMessageBox.Yes | QMessageBox.No, QMessageBox.No)
        if r != QMessageBox.Yes:
            return
        bat = Path(tempfile.gettempdir()) / "thinstt_uninstall.bat"
        lines = [
            "@echo off",
            f"set PID={os.getpid()}",
            ":wait",
            'tasklist /FI "PID eq %PID%" | find "%PID%" >nul',
            "if not errorlevel 1 (timeout /t 1 /nobreak >nul & goto wait)",
            f'rmdir /s /q "{config.app_data_dir()}"',
        ]
        lines += [f'rmdir /s /q "{m.cache_dir}"' for m in models.MODELS]
        lines += [f'rmdir /s /q "{APP_ROOT}"', 'del "%~f0"']
        bat.write_text("\r\n".join(lines) + "\r\n", encoding="ascii")
        subprocess.Popen(["cmd", "/c", str(bat)],
                         creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
        QApplication.quit()

    # ---------- tray ----------
    def _setup_tray(self):
        self.tray = QSystemTrayIcon(self)
        self.tray.setIcon(self.style().standardIcon(QStyle.SP_MediaVolume))
        menu = QMenu()
        show = QAction("Show", self)
        show.triggered.connect(self.showNormal)
        quit_a = QAction("Quit", self)
        quit_a.triggered.connect(QApplication.quit)
        menu.addAction(show)
        menu.addAction(quit_a)
        self.tray.setContextMenu(menu)
        self.tray.setToolTip("Thin STT")
        self.tray.show()

    def closeEvent(self, e):
        e.ignore()
        self.hide()
        self.tray.showMessage("Thin STT", "Still running — hotkeys active.",
                              QSystemTrayIcon.Information, 2000)


def main():
    app = QApplication(sys.argv)
    app.setQuitOnLastWindowClosed(False)
    app.setStyleSheet(STYLESHEET)
    w = MainWindow()
    w.show()
    sys.exit(app.exec())


if __name__ == "__main__":
    main()
