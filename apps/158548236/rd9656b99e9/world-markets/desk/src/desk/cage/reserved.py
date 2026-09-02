from __future__ import annotations

import re

from desk.cage.slots import RESERVED_ACTIVE
from desk.cage.types import CageState

RESERVED_WORDS = ("done", "off", "hold", "cancel", "stop", "repeat", "slower", "show me")

_WORD = re.compile(
    r"^\s*(?:(?:please|just)\s+)?(done|off|hold|cancel|stop|repeat|slower|show me)"
    r"(?:\s+(?:please|that|it|the ticket))?[\s.!?]*$",
    re.IGNORECASE,
)

SOFT_AFFIRMATIVES = re.compile(
    r"^\s*(yes|yeah|yep|yup|sure|ok|okay|go ahead|do it|place it|send it)\b",
    re.IGNORECASE,
)


def detect_reserved(text: str) -> str | None:
    """Return the reserved word if the whole final transcript matches the tiny grammar."""
    cleaned = " ".join(text.strip().lower().split())
    if not cleaned:
        return None
    match = _WORD.match(cleaned)
    if not match:
        return None
    word = match.group(1).lower()
    return word


def reserved_is_active(word: str, state: CageState) -> bool:
    active = RESERVED_ACTIVE.get(word, frozenset())
    return state in active


def is_soft_affirmative(text: str) -> bool:
    cleaned = " ".join(text.strip().lower().split())
    if detect_reserved(cleaned):
        return False
    return bool(SOFT_AFFIRMATIVES.match(cleaned))
