"""Microphone capture -> 16 kHz mono PCM16 WAV."""

import wave
from pathlib import Path

import numpy as np

SAMPLE_RATE = 16000


class Recorder:
    def __init__(self, device=None):
        self.device = device
        self._stream = None
        self._frames: list[np.ndarray] = []

    @property
    def recording(self) -> bool:
        return self._stream is not None

    def start(self) -> None:
        import sounddevice as sd

        if self.recording:
            return
        self._frames = []
        self._stream = sd.InputStream(
            samplerate=SAMPLE_RATE, channels=1, dtype="float32",
            device=self.device,
            callback=lambda indata, frames, t, status: self._frames.append(indata.copy()),
        )
        self._stream.start()

    def stop(self, out_path: Path) -> Path:
        """Stop recording and write a PCM16 WAV. Raises if nothing captured."""
        if not self.recording:
            raise RuntimeError("not recording")
        self._stream.stop()
        self._stream.close()
        self._stream = None
        if not self._frames:
            raise RuntimeError("no audio captured")
        audio = np.concatenate(self._frames, axis=0).flatten()
        audio = np.clip(audio, -1.0, 1.0)
        pcm = (audio * 32767).astype(np.int16)
        out_path.parent.mkdir(parents=True, exist_ok=True)
        with wave.open(str(out_path), "wb") as w:
            w.setnchannels(1)
            w.setsampwidth(2)
            w.setframerate(SAMPLE_RATE)
            w.writeframes(pcm.tobytes())
        return out_path

    def abort(self) -> None:
        if self.recording:
            self._stream.stop()
            self._stream.close()
            self._stream = None
        self._frames = []


def input_devices() -> list[tuple[int, str]]:
    import sounddevice as sd
    out = []
    try:
        for i, d in enumerate(sd.query_devices()):
            if d["max_input_channels"] > 0:
                out.append((i, d["name"]))
    except Exception:
        pass
    return out
