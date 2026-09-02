from __future__ import annotations

from pathlib import Path
from typing import Any


_WORD_SPLIT = None


class InterruptionTracker:
    """Map TTS character offsets to word index. Cage uses completion; tape uses heard_up_to."""

    def __init__(self) -> None:
        self.text = ""
        self.words: list[str] = []
        self.char_to_word: list[int] = []
        self.heard_up_to_word = -1
        self.playing = False
        self.completed = False

    def start(self, text: str) -> None:
        self.text = text
        self.words = [w for w in text.replace("—", " ").split() if w]
        self.char_to_word = []
        idx = 0
        for wi, word in enumerate(self.words):
            pos = text.find(word, idx)
            if pos < 0:
                pos = idx
            for _ in range(pos - len(self.char_to_word)):
                self.char_to_word.append(max(0, wi - 1) if wi else 0)
            for _ in word:
                self.char_to_word.append(wi)
            idx = pos + len(word)
        while len(self.char_to_word) < len(text):
            self.char_to_word.append(len(self.words) - 1 if self.words else 0)
        self.heard_up_to_word = -1
        self.playing = True
        self.completed = False

    def on_marker(self, char_offset: int) -> int:
        if not self.char_to_word:
            return -1
        clamped = min(max(0, char_offset), len(self.char_to_word) - 1)
        self.heard_up_to_word = self.char_to_word[clamped]
        return self.heard_up_to_word

    def complete(self) -> None:
        self.playing = False
        self.completed = True
        self.heard_up_to_word = len(self.words) - 1 if self.words else -1

    def barge_in(self) -> dict[str, Any]:
        last = self.heard_up_to_word
        heard = " ".join(self.words[: last + 1]) if last >= 0 else ""
        self.playing = False
        return {
            "heard_up_to_word": last,
            "heard_text": heard,
            "completed": self.completed,
        }

    def context_note(self) -> str:
        info = self.barge_in() if self.playing else {
            "heard_text": " ".join(self.words[: self.heard_up_to_word + 1]) if self.heard_up_to_word >= 0 else "",
        }
        heard = info.get("heard_text") or ""
        return f'[interrupted after: "...«{heard}»"]' if heard else "[interrupted]"
