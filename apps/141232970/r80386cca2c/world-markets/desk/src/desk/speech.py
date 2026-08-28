"""Number and ticker normalizer. Every TTS string passes through speak_text()."""

from __future__ import annotations

import re
from decimal import Decimal, ROUND_HALF_UP
from typing import Literal

ONES = [
    "zero",
    "one",
    "two",
    "three",
    "four",
    "five",
    "six",
    "seven",
    "eight",
    "nine",
    "ten",
    "eleven",
    "twelve",
    "thirteen",
    "fourteen",
    "fifteen",
    "sixteen",
    "seventeen",
    "eighteen",
    "nineteen",
]
TENS = [
    "",
    "",
    "twenty",
    "thirty",
    "forty",
    "fifty",
    "sixty",
    "seventy",
    "eighty",
    "ninety",
]

Verbosity = Literal["novice", "expert"]

_MONEY = re.compile(r"\$(\d{1,3}(?:,\d{3})+|\d+)(?:\.(\d{1,2}))?")
_PLAIN_MONEY = re.compile(r"\b(\d{1,6})(?:\.(\d{1,2}))?\s*(?:USD|USDT|dollars?)\b", re.I)
_TICKER_TOKEN = re.compile(r"\b([A-Z]{1,6})(?:\.([A-Z]{1,4}))?\b")
_SPELLED = re.compile(r"\[spell:([A-Za-z0-9]+)\]")
_DIGITS_HINT = re.compile(r"\[digits:(\d+)\]")


def _join(*parts: str) -> str:
    return " ".join(p for p in parts if p)


def speak_int(n: int) -> str:
    if n < 0:
        return "minus " + speak_int(-n)
    if n < 20:
        return ONES[n]
    if n < 100:
        tens, ones = divmod(n, 10)
        return TENS[tens] if ones == 0 else f"{TENS[tens]}-{ONES[ones]}"
    if n < 1000:
        hundreds, rest = divmod(n, 100)
        if rest == 0:
            return f"{ONES[hundreds]} hundred"
        if rest < 10:
            return f"{ONES[hundreds]} oh {ONES[rest]}"
        return _join(f"{ONES[hundreds]} hundred", speak_int(rest))
    if n < 1_000_000:
        thousands, rest = divmod(n, 1000)
        head = speak_int(thousands) + " thousand"
        return head if rest == 0 else _join(head, speak_int(rest))
    millions, rest = divmod(n, 1_000_000)
    head = speak_int(millions) + " million"
    return head if rest == 0 else _join(head, speak_int(rest))


def speak_grouped_int(n: int) -> str:
    """Desk readback grouping: 185 → 'one eighty-five'; 3800 → 'thirty-eight hundred'."""
    if n < 0:
        return "minus " + speak_grouped_int(-n)
    if n < 100:
        return speak_int(n)
    if n < 1000:
        hundreds, rest = divmod(n, 100)
        if rest == 0:
            return f"{ONES[hundreds]} hundred"
        return _join(ONES[hundreds], speak_int(rest) if rest >= 10 else f"oh {ONES[rest]}")
    if n < 10_000 and n % 100 == 0:
        if n % 1000 == 0:
            return speak_int(n // 1000) + " thousand"
        hundreds = n // 100
        return f"{speak_int(hundreds)} hundred"
    if n < 1_000_000:
        thousands, rest = divmod(n, 1000)
        head = speak_int(thousands) + " thousand"
        if rest == 0:
            return head
        if rest < 100:
            return _join(head, speak_int(rest))
        return _join(head, speak_grouped_int(rest))
    return speak_int(n)


def _cents(frac: Decimal) -> int:
    return int((frac * 100).quantize(Decimal("1"), rounding=ROUND_HALF_UP))


def speak_price(amount: Decimal, *, verbosity: Verbosity, currency: str = "dollars") -> str:
    amount = amount.quantize(Decimal("0.01"), rounding=ROUND_HALF_UP)
    negative = amount < 0
    amount = abs(amount)
    dollars = int(amount)
    cents = _cents(amount - Decimal(dollars))
    if verbosity == "novice":
        spoken = speak_int(dollars) + f" {currency}"
        if cents:
            spoken += " and " + speak_int(cents) + " cents"
        return ("minus " + spoken) if negative else spoken
    body = speak_grouped_int(dollars)
    if cents:
        if cents < 10:
            body = _join(body, "oh", ONES[cents])
        else:
            body = _join(body, speak_int(cents))
    return ("minus " + body) if negative else body


def speak_decimal(value: Decimal, *, verbosity: Verbosity = "expert") -> str:
    value = Decimal(str(value))
    if value == value.to_integral_value():
        return speak_int(int(value)) if verbosity == "novice" else speak_grouped_int(int(value))
    sign = "minus " if value < 0 else ""
    value = abs(value)
    whole = int(value)
    frac = str(value - whole).split(".")[-1].rstrip("0")
    if whole == 0:
        digits = " ".join(ONES[int(d)] for d in frac)
        return f"{sign}point {digits}"
    return f"{sign}{speak_grouped_int(whole) if verbosity == 'expert' else speak_int(whole)} point {' '.join(ONES[int(d)] for d in frac)}"


def spell_ticker(symbol: str) -> str:
    return " ".join(ch.upper() for ch in symbol if ch.isalnum())


def digit_group(n: int) -> str:
    return " ".join(ONES[int(ch)] if ch.isdigit() else ch for ch in str(n))


def speak_quantity(
    value: Decimal,
    *,
    unit: str,
    verbosity: Verbosity,
    confidence: float,
    readback: bool,
) -> str:
    if value >= 1000 and readback and confidence < 0.9:
        n = int(value)
        return f"{speak_int(n)} — that's {digit_group(n).replace(' ', '-')} — {unit}"
    if value == Decimal("0.5"):
        qty = "one half" if verbosity == "novice" else "point five"
        return f"{qty} {unit}"
    return f"{speak_decimal(value, verbosity=verbosity)} {unit}"


def _rewrite_money(match: re.Match[str], verbosity: Verbosity) -> str:
    dollars = match.group(1).replace(",", "")
    cents = match.group(2) or "0"
    amount = Decimal(dollars) + Decimal(cents) / Decimal(10 ** len(cents) if cents != "0" else 1)
    if match.group(2) is not None:
        amount = Decimal(f"{dollars}.{cents}")
    else:
        amount = Decimal(dollars)
    return speak_price(amount, verbosity=verbosity)


def speak_text(
    text: str,
    *,
    verbosity: Verbosity = "expert",
    state: str | None = None,
    slot_confidence: dict[str, float] | None = None,
    spell_tickers: frozenset[str] | None = None,
) -> str:
    """Normalize numbers, money, and tickers for TTS. Idempotent on already-spoken text."""
    slot_confidence = slot_confidence or {}
    spell_tickers = spell_tickers or frozenset()
    out = text

    def digits_sub(match: re.Match[str]) -> str:
        return digit_group(int(match.group(1))).replace(" ", "-")

    out = _DIGITS_HINT.sub(digits_sub, out)
    out = _SPELLED.sub(lambda m: spell_ticker(m.group(1)), out)
    out = _MONEY.sub(lambda m: _rewrite_money(m, verbosity), out)
    out = _PLAIN_MONEY.sub(
        lambda m: speak_price(
            Decimal(m.group(1) + (("." + m.group(2)) if m.group(2) else "")),
            verbosity=verbosity,
        ),
        out,
    )

    if spell_tickers:
        def ticker_sub(match: re.Match[str]) -> str:
            token = match.group(1)
            if token in spell_tickers:
                return spell_ticker(token)
            return match.group(0)

        out = _TICKER_TOKEN.sub(ticker_sub, out)

    if state == "READBACK" and slot_confidence.get("quantity", 1.0) < 0.9:
        def qty_sub(match: re.Match[str]) -> str:
            n = int(match.group(1))
            return f"{speak_int(n)} — that's {digit_group(n).replace(' ', '-')} —"

        out = re.sub(r"\b(\d{4,})\b", qty_sub, out)

    out = re.sub(r"\s+", " ", out).replace(" — ", " — ").strip()
    return out
