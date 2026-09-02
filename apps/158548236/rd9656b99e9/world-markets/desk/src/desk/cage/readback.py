from __future__ import annotations

from decimal import Decimal

from desk.cage.types import OrderDraft, SpokenText
from desk.config import DeskConfig
from desk.speech import (
    spell_ticker,
    speak_price,
    speak_quantity,
    speak_text,
)


def _unit(draft: OrderDraft) -> str:
    inst = draft.instrument
    if draft.quantity and draft.quantity.kind.value == "dollars":
        return "dollars of " + (inst.name.lower() if inst else "the name")
    if draft.quantity and draft.quantity.kind.value == "pct_of_position":
        pct = draft.quantity.value
        if pct == Decimal("0.5") or pct == Decimal("50"):
            return "half the position in " + (inst.name.lower() if inst else "the name")
        return f"{pct} percent of the position in " + (inst.name.lower() if inst else "the name")
    if inst:
        return inst.name.lower()
    return "units"


def _consequence(
    draft: OrderDraft,
    *,
    equity: Decimal,
    notional: Decimal | None,
    spread_bps: Decimal | None,
) -> str:
    parts: list[str] = []
    if notional is not None and equity > 0:
        pct = (notional / equity) * Decimal(100)
        parts.append(f"That's {pct.quantize(Decimal('0.1'))} percent of the book")
    if spread_bps is not None and spread_bps > Decimal("20"):
        parts.append(f"the spread is {spread_bps.quantize(Decimal('1'))} basis points")
    return ". ".join(parts) + ("." if parts else "")


def render_readback(
    draft: OrderDraft,
    *,
    config: DeskConfig,
    equity: Decimal,
    notional: Decimal | None = None,
    spread_bps: Decimal | None = None,
) -> SpokenText:
    """Canonical slot order. Binding text — never LLM-authored."""
    if draft.side is None or draft.quantity is None or draft.instrument is None:
        raise ValueError("readback requires side, quantity, and instrument")
    inst = draft.instrument
    conf = draft.slot_confidence
    verbosity = config.verbosity
    spell_inst = conf.get("instrument", inst.confidence) < 0.9
    ticker = spell_ticker(inst.symbol) if spell_inst else inst.symbol
    name = inst.name
    unit = _unit(draft)
    qty_conf = conf.get("quantity", 1.0)
    if draft.quantity.kind.value == "pct_of_position":
        qty = ""
        unit_line = unit
    else:
        qty = speak_quantity(
            draft.quantity.value,
            unit=unit,
            verbosity=verbosity,
            confidence=qty_conf,
            readback=True,
        )
        unit_line = qty

    side = "Buy" if draft.side == "buy" else "Sell"
    product = inst.product
    order_type = draft.order_type or "market"
    price_bit = ""
    if order_type in {"limit", "stop_limit"} and draft.limit_price is not None:
        price_bit = "limit " + speak_price(draft.limit_price, verbosity=verbosity)
    if order_type in {"stop", "stop_limit"} and draft.stop_price is not None:
        stop_bit = "stop " + speak_price(draft.stop_price, verbosity=verbosity)
        price_bit = (price_bit + " " + stop_bit).strip()
    if order_type == "market":
        price_bit = "market"

    duration = "good for the day" if draft.duration == "day" else "good till canceled"
    if spell_inst:
        inst_bit = f"{name}, {ticker}"
    else:
        inst_bit = f"{name}, {inst.symbol}"

    head = f"{side} {unit_line}, {inst_bit}, {product}, {price_bit}, {duration}."
    consequence = _consequence(draft, equity=equity, notional=notional, spread_bps=spread_bps)
    text = " ".join(p for p in (head, consequence, "Done?") if p)
    text = speak_text(
        text,
        verbosity=verbosity,
        state="READBACK",
        slot_confidence=conf,
        spell_tickers=frozenset({inst.symbol}) if spell_inst else frozenset(),
    )
    return SpokenText.from_text(text)
