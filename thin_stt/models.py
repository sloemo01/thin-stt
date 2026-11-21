"""Model registry: OpenAI Whisper + NVIDIA Parakeet, backed by onnx-asr / HF hub."""

import shutil
from dataclasses import dataclass, field
from pathlib import Path

from huggingface_hub import snapshot_download

try:
    from huggingface_hub.constants import HF_HUB_CACHE
except ImportError:
    HF_HUB_CACHE = str(Path.home() / ".cache" / "huggingface" / "hub")

_HUB_CACHE = Path(HF_HUB_CACHE)


@dataclass(frozen=True)
class Model:
    id: str                 # unique row id
    engine: str             # "whisper" | "parakeet"
    label: str              # display name
    load_name: str          # name passed to onnx_asr.load_model
    repo_id: str            # HF repo for download/delete
    approx_mb: int
    patterns: tuple[str, ...] = field(default=("*",))

    @property
    def cache_dir(self) -> Path:
        return _HUB_CACHE / ("models--" + self.repo_id.replace("/", "--"))


_WHISPER_HF = ("*.json", "*.txt",
              "**/encoder_model.onnx*", "**/decoder_model_merged.onnx*")
_PARAKEET = ("*.json", "*.txt", "*.model",
             "model.onnx*", "encoder-model.onnx*", "decoder-model.onnx*",
             "decoder_joint-model.onnx*", "nemo128.onnx*")

MODELS: list[Model] = [
    # --- OpenAI Whisper (ONNX) ---
    Model("whisper-tiny", "whisper", "Whisper Tiny", "onnx-community/whisper-tiny",
          "onnx-community/whisper-tiny", 240, _WHISPER_HF),
    Model("whisper-base", "whisper", "Whisper Base", "whisper-base",
          "istupakov/whisper-base-onnx", 240,
          ("*.json", "*.txt", "whisper-*_beamsearch.onnx*")),
    Model("whisper-small", "whisper", "Whisper Small", "onnx-community/whisper-small",
          "onnx-community/whisper-small", 1000, _WHISPER_HF),
    Model("whisper-large-v3-turbo", "whisper", "Whisper Large v3 Turbo",
          "onnx-community/whisper-large-v3-turbo", "onnx-community/whisper-large-v3-turbo", 3300,
          _WHISPER_HF),
    # --- NVIDIA Parakeet ---
    Model("parakeet-ctc", "parakeet", "Parakeet CTC 0.6B (EN)", "nemo-parakeet-ctc-0.6b",
          "istupakov/parakeet-ctc-0.6b-onnx", 640, _PARAKEET),
    Model("parakeet-rnnt", "parakeet", "Parakeet RNN-T 0.6B (EN)", "nemo-parakeet-rnnt-0.6b",
          "istupakov/parakeet-rnnt-0.6b-onnx", 640, _PARAKEET),
    Model("parakeet-tdt-v2", "parakeet", "Parakeet TDT 0.6B v2 (EN)", "nemo-parakeet-tdt-0.6b-v2",
          "istupakov/parakeet-tdt-0.6b-v2-onnx", 1300, _PARAKEET),
    Model("parakeet-tdt-v3", "parakeet", "Parakeet TDT 0.6B v3 (multilingual)", "nemo-parakeet-tdt-0.6b-v3",
          "istupakov/parakeet-tdt-0.6b-v3-onnx", 2500, _PARAKEET),
]

BY_ID = {m.id: m for m in MODELS}


def is_downloaded(model: Model) -> bool:
    snaps = model.cache_dir / "snapshots"
    if not snaps.is_dir():
        return False
    for snap in snaps.iterdir():
        if any(snap.rglob("*.onnx")):
            return True
    return False


def disk_size_mb(model: Model) -> int:
    snaps = model.cache_dir / "snapshots"
    if not snaps.is_dir():
        return 0
    seen: set[Path] = set()
    total = 0
    for p in snaps.rglob("*"):
        if not p.is_file():
            continue
        try:
            resolved = p.resolve()
            if resolved in seen:
                continue
            seen.add(resolved)
            total += resolved.stat().st_size
        except OSError:
            pass
    return total // (1024 * 1024)


def remote_size_bytes(model: Model) -> int | None:
    """Sum of the files our allow_patterns would download."""
    import fnmatch
    from huggingface_hub import HfApi
    info = HfApi().model_info(model.repo_id, files_metadata=True)
    total = 0
    for s in info.siblings or []:
        if s.size and any(fnmatch.fnmatch(s.rfilename, p) for p in model.patterns):
            total += s.size
    return total or None


def download(model: Model, progress_cb=None) -> Path:
    """Download required files into the HF cache. progress_cb(done_bytes, total_bytes)."""
    if progress_cb is None:
        return Path(snapshot_download(model.repo_id, allow_patterns=list(model.patterns)))

    from huggingface_hub.utils import tqdm as hf_tqdm

    class _T(hf_tqdm):
        def update(self, n=1):
            super().update(n)
            try:
                progress_cb(self.n, self.total or 0)
            except Exception:
                pass

    return Path(snapshot_download(
        model.repo_id, allow_patterns=list(model.patterns), tqdm_class=_T))


def delete(model: Model) -> None:
    if model.cache_dir.is_dir():
        shutil.rmtree(model.cache_dir, ignore_errors=True)
