"""Deterministic NLU used when no LLM key is present, and as a test driver."""

from __future__ import annotations

import re
from decimal import Decimal
from typing import Any, Literal

from desk.cage.types import MandateDraft, MandateTrigger, OrderDraft, Quantity, QuantityKind

IntentName = Literal[
    "quote",
    "positions",
    "order",
    "mandate",
    "simulate",
    "arm",
    "rationale",
    "list_mandates",
    "suspend",
    "revoke",
    "open",
    "open_rushed",
    "watchlist_add",
    "watchlist_remove",
    "journal",
    "resume_ticket",
    "unknown",
]

ONES = {
    "zero": 0,
    "oh": 0,
    "one": 1,
    "two": 2,
    "three": 3,
    "four": 4,
    "five": 5,
    "six": 6,
    "seven": 7,
    "eight": 8,
    "nine": 9,
    "ten": 10,
    "eleven": 11,
    "twelve": 12,
    "thirteen": 13,
    "fourteen": 14,
    "fifteen": 15,
    "sixteen": 16,
    "seventeen": 17,
    "eighteen": 18,
    "nineteen": 19,
}
TENS = {
    "twenty": 20,
    "thirty": 30,
    "forty": 40,
    "fifty": 50,
    "sixty": 60,
    "seventy": 70,
    "eighty": 80,
    "ninety": 90,
}
SCALES = {"hundred": 100, "thousand": 1000, "million": 1_000_000}


def _tokenize(text: str) -> list[str]:
    cleaned = text.lower().replace(",", " ").replace("'", "")
    return re.findall(r"[a-z0-9.]+", cleaned)


def parse_number_words(tokens: list[str]) -> tuple[Decimal | None, int]:
    """Parse a leading number (words or digits). Returns (value, tokens_consumed)."""
    if not tokens:
        return None, 0
    if re.fullmatch(r"\d+(?:\.\d+)?", tokens[0]):
        return Decimal(tokens[0]), 1
    if tokens[0] in {"a", "an"}:
        rest, n = parse_number_words(tokens[1:])
        if rest is not None:
            return rest, n + 1
        return Decimal(1), 1
    if tokens[0] == "half":
        return Decimal("0.5"), 1
    if tokens[0] == "point" and len(tokens) > 1 and tokens[1] in ONES:
        return Decimal("0." + str(ONES[tokens[1]])), 2
    # "two tenths"
    if len(tokens) >= 2 and tokens[0] in ONES and tokens[1] in {"tenth", "tenths"}:
        return Decimal(ONES[tokens[0]]) / Decimal(10), 2
    total = 0
    current = 0
    consumed = 0
    started = False
    i = 0
    while i < len(tokens):
        w = tokens[i]
        if w in {"and", "-"}:
            i += 1
            consumed += 1
            continue
        if w in ONES:
            current += ONES[w]
            started = True
            i += 1
            consumed += 1
            continue
        if w in TENS:
            current += TENS[w]
            started = True
            i += 1
            consumed += 1
            continue
        if w in SCALES:
            if current == 0:
                current = 1
            current *= SCALES[w]
            total += current
            current = 0
            started = True
            i += 1
            consumed += 1
            continue
        break
    if not started:
        return None, 0
    return Decimal(total + current), consumed


def _cut_instrument(tokens: list[str]) -> tuple[str, str | None, list[str]]:
    """Return (query, product, remaining_after). Stops at limit/stop/market/day."""
    stop = {"limit", "stop", "market", "day", "gtc", "done", "if"}
    product = None
    out: list[str] = []
    i = 0
    while i < len(tokens):
        t = tokens[i]
        if t in stop:
            break
        if t in {"perp", "perpetual"}:
            product = "perp"
            i += 1
            continue
        if t == "spot":
            product = "spot"
            i += 1
            continue
        if t in {"of", "the"}:
            i += 1
            continue
        out.append(t)
        i += 1
    return " ".join(out).strip(), product, tokens[i:]


class Intent:
    def __init__(self, name: IntentName, **fields: Any) -> None:
        self.name = name
        self.fields = fields

    def __repr__(self) -> str:
        return f"Intent({self.name!r}, {self.fields})"


def parse_utterance(text: str) -> Intent:
    raw = text.strip()
    tokens = _tokenize(raw)
    if not tokens:
        return Intent("unknown")

    joined = " ".join(tokens)

    if any(p in joined for p in ("i'm rushed", "im rushed", "just the headlines", "skip the liturgy")):
        return Intent("open_rushed")
    if joined in {"the open", "morning", "what's going on", "whats going on", "good morning"} or joined.startswith(
        "the open"
    ):
        return Intent("open")
    if "what's going on" in joined or "whats going on" in joined:
        return Intent("open")

    if tokens[0] in {"resume", "ticket"} or joined in {"the ticket", "show the ticket"}:
        return Intent("resume_ticket")

    if joined.startswith("list mandates") or joined in {"what's armed", "whats armed", "registry"}:
        return Intent("list_mandates")
    if tokens[0] == "suspend":
        hours = 24
        n, c = parse_number_words(tokens[1:])
        if n is not None:
            hours = int(n)
        return Intent("suspend", hours=hours)
    if tokens[0] in {"revoke", "kill"} and "mandate" in joined or joined.startswith("revoke"):
        return Intent("revoke")

    if joined.startswith("simulate") or "run the simulation" in joined or joined in {"simulate it", "yes simulate"}:
        return Intent("simulate")
    if joined in {"arm it", "arm", "arm the rule"}:
        return Intent("arm")
    if joined.startswith("because") or joined.startswith("rationale"):
        reason = re.sub(r"^(because|rationale)\s+", "", raw, flags=re.I)
        return Intent("rationale", text=reason)

    m_add = re.match(r"^(?:add|watch)\s+(.+?)(?:\s+to (?:the )?watchlist)?$", joined)
    if m_add and "watchlist" in joined:
        return Intent("watchlist_add", query=m_add.group(1).replace(" to the watchlist", "").replace(" to watchlist", ""))
    if joined.startswith("remove ") and "watchlist" in joined:
        q = joined.replace("remove", "").replace("from the watchlist", "").replace("from watchlist", "").strip()
        return Intent("watchlist_remove", query=q)

    if "positions" in joined or joined in {"what's my book", "whats my book", "the book"}:
        return Intent("positions")

    quote_m = re.match(
        r"^(?:what(?:'s|s| is)\s+)?(.+?)\s+(?:doing|trading at|at|mark|price)\??$",
        joined,
    )
    if quote_m and tokens[0] in {"what", "whats", "how's", "hows", "quote"}:
        q = quote_m.group(1)
        q = re.sub(r"^(is|the)\s+", "", q)
        return Intent("quote", query=q)
    if tokens[0] == "quote" and len(tokens) > 1:
        return Intent("quote", query=" ".join(tokens[1:]))

    # mandate: if X drops below P, sell half
    if tokens[0] == "if":
        return _parse_mandate(tokens[1:], raw)

    if tokens[0] in {"buy", "sell", "long", "short"}:
        return _parse_order(tokens, raw)

    if joined.startswith("note ") or joined.startswith("journal "):
        return Intent("journal", text=raw.split(" ", 1)[-1])

    return Intent("unknown", text=raw)


def _parse_order(tokens: list[str], raw: str) -> Intent:
    side = "buy" if tokens[0] in {"buy", "long"} else "sell"
    rest = tokens[1:]
    product = "perp" if tokens[0] in {"long", "short"} else None
    qty: Quantity | None = None
    n, consumed = parse_number_words(rest)
    rest = rest[consumed:]
    kind = QuantityKind.BASE
    if rest and rest[0] in {"dollars", "dollar", "bucks", "usdt"}:
        kind = QuantityKind.DOLLARS
        rest = rest[1:]
        if rest and rest[0] == "of":
            rest = rest[1:]
    elif rest and rest[0] in {"percent", "pct", "%"}:
        kind = QuantityKind.PCT_OF_POSITION
        rest = rest[1:]
    elif n == Decimal("0.5") and (not rest or rest[0] in {"the", "of"}):
        kind = QuantityKind.PCT_OF_POSITION
    if n is not None:
        qty = Quantity(kind=kind, value=n)
    if rest and rest[0] in {"shares", "share", "units", "contracts"}:
        rest = rest[1:]
        if rest and rest[0] == "of":
            rest = rest[1:]
    query, prod2, rest = _cut_instrument(rest)
    product = prod2 or product
    order_type = "market"
    limit = None
    stop = None
    duration = "day"
    i = 0
    while i < len(rest):
        t = rest[i]
        if t == "limit":
            order_type = "limit"
            val, n = parse_number_words(rest[i + 1 :])
            if val is not None:
                limit = val
                i += 1 + n
                continue
        if t == "stop":
            order_type = "stop" if order_type != "limit" else "stop_limit"
            val, n = parse_number_words(rest[i + 1 :])
            if val is not None:
                stop = val
                i += 1 + n
                continue
        if t == "market":
            order_type = "market"
        if t == "gtc":
            duration = "gtc"
        i += 1
    conf = {}
    if qty:
        conf["quantity"] = 0.95
    if query:
        conf["instrument"] = 0.5  # resolver overwrites
    draft = OrderDraft(
        side=side,
        quantity=qty,
        instrument_query=query or None,
        order_type=order_type,
        limit_price=limit,
        stop_price=stop,
        duration=duration,
        slot_confidence=conf,
    )
    return Intent("order", draft=draft, product=product, query=query)


def _parse_mandate(tokens: list[str], raw: str) -> Intent:
    # X drops below P, sell half
    cmp_map = {
        ("drops", "below"): "lt",
        ("falls", "below"): "lt",
        ("goes", "under"): "lt",
        ("rises", "above"): "gt",
        ("breaks", "above"): "gt",
        ("goes", "through"): "gt",
    }
    comparator = "lt"
    inst_tokens: list[str] = []
    i = 0
    while i < len(tokens):
        pair = (tokens[i], tokens[i + 1] if i + 1 < len(tokens) else "")
        if pair in cmp_map:
            comparator = cmp_map[pair]
            i += 2
            break
        if tokens[i] in {"below", "under"}:
            comparator = "lt"
            i += 1
            break
        if tokens[i] in {"above", "over"}:
            comparator = "gt"
            i += 1
            break
        inst_tokens.append(tokens[i])
        i += 1
    price, n = parse_number_words(tokens[i:])
    rest = tokens[i + n :]
    if rest and rest[0] in {"then", ","}:
        rest = rest[1:]
    action_intent = _parse_order(rest, raw) if rest and rest[0] in {"buy", "sell", "long", "short"} else None
    action = action_intent.fields["draft"] if action_intent else OrderDraft(
        side="sell",
        quantity=Quantity(kind=QuantityKind.PCT_OF_POSITION, value=Decimal("0.5")),
        order_type="limit",
        duration="day",
        slot_confidence={"quantity": 0.95},
    )
    query = " ".join(inst_tokens).replace(" it ", " ").strip() or "it"
    mandate = MandateDraft(
        trigger=MandateTrigger(instrument_query=query, comparator=comparator, price=price),
        action=action,
    )
    return Intent("mandate", mandate=mandate, query=query)
