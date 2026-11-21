"""Lazy onnx-asr transcription."""

from pathlib import Path


class Transcriber:
    def __init__(self):
        self._models = {}

    def transcribe(self, model_id: str, load_name: str, wav_path: Path) -> str:
        if model_id not in self._models:
            import onnx_asr
            self._models[model_id] = onnx_asr.load_model(load_name)
        return self._models[model_id].recognize(str(wav_path)).strip()

    def drop(self, model_id: str) -> None:
        self._models.pop(model_id, None)

    def drop_all(self) -> None:
        self._models.clear()
