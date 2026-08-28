"""Turn-based speech-to-text. Deepgram first, Whisper if that key is absent."""

from __future__ import annotations

import os
from typing import Any

import httpx


class SttError(RuntimeError):
    pass


def transcribe(audio: bytes, mime: str | None = None) -> str:
    if not audio:
        raise SttError("empty audio")
    content_type = _content_type(audio, mime)
    deepgram = os.getenv("DEEPGRAM_API_KEY", "").strip()
    if deepgram:
        return _deepgram(audio, content_type, deepgram)
    openai = os.getenv("OPENAI_API_KEY", "").strip()
    if openai:
        return _whisper(audio, content_type, openai)
    raise SttError(
        "speech recognition is not configured — set DEEPGRAM_API_KEY or OPENAI_API_KEY"
    )


def _deepgram(audio: bytes, content_type: str, key: str) -> str:
    try:
        response = httpx.post(
            "https://api.deepgram.com/v1/listen",
            params={"model": "nova-2", "smart_format": "true"},
            headers={"Authorization": f"Token {key}", "Content-Type": content_type},
            content=audio,
            timeout=60.0,
        )
    except httpx.HTTPError as exc:
        raise SttError(f"deepgram is not reachable ({exc})") from exc
    if response.is_error:
        raise SttError(f"deepgram rejected the audio (HTTP {response.status_code})")
    data: dict[str, Any] = response.json()
    text = (
        ((data.get("results") or {}).get("channels") or [{}])[0]
        .get("alternatives", [{}])[0]
        .get("transcript")
        or ""
    )
    text = str(text).strip()
    if not text:
        raise SttError("deepgram returned an empty transcript")
    return text


def _content_type(audio: bytes, mime: str | None) -> str:
    if len(audio) >= 12 and audio.startswith(b"RIFF") and audio[8:12] == b"WAVE":
        return "audio/wav"
    if audio.startswith(b"OggS"):
        return "audio/ogg"
    if len(audio) >= 4 and audio[:4] == b"\x1a\x45\xdf\xa3":
        return "audio/webm"
    return mime or "audio/webm"


def _whisper(audio: bytes, content_type: str, key: str) -> str:
    ext = "webm"
    if "ogg" in content_type:
        ext = "ogg"
    elif "mp4" in content_type or "m4a" in content_type:
        ext = "m4a"
    elif "mpeg" in content_type or "mp3" in content_type:
        ext = "mp3"
    elif "wav" in content_type:
        ext = "wav"
    try:
        response = httpx.post(
            "https://api.openai.com/v1/audio/transcriptions",
            headers={"Authorization": f"Bearer {key}"},
            data={"model": "whisper-1"},
            files={"file": (f"note.{ext}", audio, content_type)},
            timeout=60.0,
        )
    except httpx.HTTPError as exc:
        raise SttError(f"whisper is not reachable ({exc})") from exc
    if response.is_error:
        raise SttError(f"whisper rejected the audio (HTTP {response.status_code})")
    data = response.json()
    text = str(data.get("text") or "").strip()
    if not text:
        raise SttError("whisper returned an empty transcript")
    return text
