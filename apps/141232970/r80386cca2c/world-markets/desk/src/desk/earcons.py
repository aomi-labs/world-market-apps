"""Two calm earcons — fill and chime. Generated locally; no vendor samples."""

from __future__ import annotations

import math
import struct
import wave
from pathlib import Path


def _tone(path: Path, freqs: list[float], ms: int = 180, volume: float = 0.18) -> None:
    rate = 22050
    n = int(rate * ms / 1000)
    with wave.open(str(path), "w") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(rate)
        frames = bytearray()
        for i in range(n):
            t = i / rate
            env = min(1.0, i / 400) * min(1.0, (n - i) / 800)
            sample = sum(math.sin(2 * math.pi * f * t) for f in freqs) / len(freqs)
            val = int(max(-1, min(1, sample * env * volume)) * 32767)
            frames += struct.pack("<h", val)
        w.writeframes(bytes(frames))


def ensure_earcons(directory: Path) -> None:
    directory.mkdir(parents=True, exist_ok=True)
    fill = directory / "fill.wav"
    chime = directory / "chime.wav"
    if not fill.exists():
        _tone(fill, [220, 330], ms=220)
    if not chime.exists():
        _tone(chime, [392, 494], ms=280)
