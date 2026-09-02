from __future__ import annotations

import re
from decimal import Decimal
from typing import Any

from desk.config import DeskConfig
from desk.speech import speak_price

SIGN_OFF = "That's the open. I'm here."
RUSHED_SIGN_OFF = "That's the tape. I'm here."


def _words(text: str) -> list[str]:
    return [w for w in re.findall(r"[A-Za-z0-9']+", text) if w]


def _clip(text: str, limit: int) -> str:
    words = _words(text)
    if len(words) <= limit:
        return text.strip()
    kept = []
    count = 0
    for token in text.split():
        n = len(_words(token))
        if count + n > limit:
            break
        kept.append(token)
        count += n
    return " ".join(kept).rstrip(".,;") + "."


def contains_day_pnl(text: str) -> bool:
    lowered = text.lower()
    return bool(
        re.search(r"\b(day p&l|day pnl|today's (pnl|p&l)|session p&l|you're up|you're down)\b", lowered)
    )


def render_open(bundle: dict[str, Any], *, config: DeskConfig, rushed: bool = False) -> str:
    world = bundle.get("world") or "Markets are quiet."
    names = bundle.get("names") or "Your names are unchanged."
    mandates = bundle.get("mandates") or "No mandate fired overnight."
    decisions = bundle.get("decisions") or "Nothing needs a decision from you."
    if rushed:
        text = f"{world} {names} {RUSHED_SIGN_OFF}"
        text = _clip(text, 25)
        if not text.endswith(RUSHED_SIGN_OFF) and RUSHED_SIGN_OFF.lower() not in text.lower():
            # preserve sign-off
            body = _clip(f"{world} {names}", 20)
            text = f"{body} {RUSHED_SIGN_OFF}"
            text = _clip(text, 25) if len(_words(text)) > 25 else text
        if contains_day_pnl(text):
            text = re.sub(r"[^.]*p&l[^.]*\.", "", text, flags=re.I)
        return " ".join(text.split())
    parts = [world, names, mandates, decisions, SIGN_OFF]
    text = " ".join(p.strip() for p in parts if p)
    text = re.sub(r"\s+", " ", text).strip()
    if contains_day_pnl(text):
        text = re.sub(r"[^.]*((day )?(p&l|pnl)|you're up|you're down)[^.]*\.", "", text, flags=re.I)
        text = " ".join(text.split())
    if len(_words(text)) > 75:
        budget = 75 - len(_words(SIGN_OFF))
        head = _clip(" ".join(parts[:4]), budget)
        text = f"{head} {SIGN_OFF}"
    if not text.endswith(SIGN_OFF.split(".")[0].split()[-1] + ".") and SIGN_OFF not in text:
        text = _clip(text, 75 - len(_words(SIGN_OFF))) + " " + SIGN_OFF
    # hard enforce
    if SIGN_OFF not in text:
        text = text.rstrip(".") + ". " + SIGN_OFF
    if len(_words(text)) > 75:
        without = text[: -len(SIGN_OFF)].strip()
        text = _clip(without, 75 - len(_words(SIGN_OFF))) + " " + SIGN_OFF
    _ = config
    return " ".join(text.split())


def default_bundle(
    *,
    quotes: list[Any],
    watchlist: list[str],
    mandate_notes: str | None,
    decision: str | None,
) -> dict[str, Any]:
    if quotes:
        q0 = quotes[0]
        mark = getattr(q0, "mark", None)
        spoken = speak_price(Decimal(str(mark)), verbosity="expert") if mark is not None else "the mark"
        world = f"Majors are mixed. {q0.symbol} is {spoken} as of seconds ago."
    else:
        world = "World marks are on the live book."
    if quotes:
        bits = []
        for q in quotes[:3]:
            bits.append(f"{q.symbol} {speak_price(Decimal(str(q.mark)), verbosity='expert')}")
        names = "On your names: " + "; ".join(bits) + "."
    else:
        names = "Watchlist is " + ", ".join(watchlist) + "." if watchlist else "No names on the watchlist."
    mandates = mandate_notes or "Mandates were quiet overnight."
    decisions = decision or "No decision item."
    return {
        "world": world,
        "names": names,
        "mandates": mandates,
        "decisions": decisions,
    }
