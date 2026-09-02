from __future__ import annotations

from decimal import Decimal
from typing import Protocol

from desk.cage.slots import instrument_ready, missing_order_slots
from desk.cage.types import (
    MandateDraft,
    OrderDraft,
    PortfolioSnapshot,
    QuantityKind,
    ValidationIssue,
)
from desk.config import DeskConfig


class PolicyMandate(Protocol):
    """Aomi policy document — not a Desk trigger mandate."""

    def allows(self, draft: OrderDraft, notional: Decimal) -> ValidationIssue | None: ...


def _notional(draft: OrderDraft, mark: Decimal | None) -> Decimal | None:
    if draft.quantity is None:
        return None
    qty = draft.quantity
    if qty.kind is QuantityKind.DOLLARS:
        return qty.value
    if qty.kind is QuantityKind.PCT_OF_POSITION:
        return None
    if mark is None:
        return None
    return qty.value * mark


def resolve_base_quantity(
    draft: OrderDraft,
    *,
    mark: Decimal | None,
    position_qty: Decimal,
) -> tuple[Decimal | None, ValidationIssue | None]:
    if draft.quantity is None:
        return None, ValidationIssue(
            slot="quantity",
            code="missing_quantity",
            message="quantity required",
            spoken="I still need a size.",
        )
    qty = draft.quantity
    if qty.value <= 0:
        return None, ValidationIssue(
            slot="quantity",
            code="non_positive",
            message="quantity must be > 0",
            spoken="Size has to be greater than zero.",
        )
    if qty.kind is QuantityKind.BASE:
        return qty.value, None
    if qty.kind is QuantityKind.DOLLARS:
        if mark is None or mark <= 0:
            return None, ValidationIssue(
                slot="quantity",
                code="no_mark",
                message="cannot convert dollars without a mark",
                spoken="I don't have a mark to convert that dollar size.",
            )
        return (qty.value / mark), None
    # pct of position
    pct = qty.value
    if pct > 1:
        pct = pct / Decimal(100)
    if position_qty == 0:
        return None, ValidationIssue(
            slot="quantity",
            code="no_position",
            message="no position to take a percentage of",
            spoken="There's no position to take a fraction of.",
        )
    return abs(position_qty) * pct, None


def validate_order(
    draft: OrderDraft,
    *,
    config: DeskConfig,
    book: PortfolioSnapshot,
    mark: Decimal | None,
    spread_bps: Decimal | None = None,
    instrument_chosen: bool = False,
    policy: PolicyMandate | None = None,
) -> list[ValidationIssue]:
    issues: list[ValidationIssue] = []
    for slot in missing_order_slots(draft):
        issues.append(
            ValidationIssue(
                slot=slot,
                code="missing_slot",
                message=f"missing {slot}",
                spoken=_spoken_missing(slot),
            )
        )
    if issues:
        return issues
    assert draft.instrument is not None
    if not instrument_ready(draft, config.instrument_confidence_threshold, instrument_chosen):
        issues.append(
            ValidationIssue(
                slot="instrument",
                code="low_confidence",
                message="instrument confidence below threshold",
                spoken="I need you to pick the name before I read it back.",
            )
        )
        return issues

    position_qty = Decimal(0)
    for pos in book.positions:
        if (
            pos.symbol == draft.instrument.symbol
            and pos.product == draft.instrument.product
        ):
            position_qty = pos.quantity
            break

    base_qty, qty_issue = resolve_base_quantity(
        draft, mark=mark, position_qty=position_qty
    )
    if qty_issue:
        issues.append(qty_issue)
        return issues
    assert base_qty is not None

    notional = _notional(draft, mark)
    if notional is None and mark is not None:
        notional = base_qty * mark

    if notional is not None and book.equity > 0:
        cap = book.equity * config.quantity_cap_pct_of_equity
        if notional > cap:
            pct = (config.quantity_cap_pct_of_equity * 100).quantize(Decimal("1"))
            issues.append(
                ValidationIssue(
                    slot="quantity",
                    code="quantity_cap",
                    message=f"notional {notional} exceeds {pct}% of equity {book.equity}",
                    spoken=(
                        f"That's above the cap — {pct} percent of the book. "
                        "I won't put that ticket in readback."
                    ),
                )
            )

    if draft.side == "buy" and notional is not None and notional > book.cash:
        issues.append(
            ValidationIssue(
                slot="quantity",
                code="buying_power",
                message="insufficient cash",
                spoken="There isn't enough cash for that size.",
            )
        )

    if policy is not None and notional is not None:
        denied = policy.allows(draft, notional)
        if denied:
            issues.append(denied)

    _ = spread_bps  # World has no extended-hours clause; spread note is readback-only.
    return issues


def validate_mandate(draft: MandateDraft, *, config: DeskConfig) -> list[ValidationIssue]:
    issues: list[ValidationIssue] = []
    trig = draft.trigger
    if trig.instrument is None:
        issues.append(
            ValidationIssue(
                slot="trigger.instrument",
                code="missing_slot",
                message="trigger instrument required",
                spoken="Which name should this rule watch?",
            )
        )
    elif trig.instrument.confidence < config.instrument_confidence_threshold:
        issues.append(
            ValidationIssue(
                slot="trigger.instrument",
                code="low_confidence",
                message="trigger instrument unresolved",
                spoken="I need a resolved name on the trigger before we arm it.",
            )
        )
    if trig.price is None:
        issues.append(
            ValidationIssue(
                slot="trigger.price",
                code="missing_slot",
                message="trigger price required",
                spoken="At what price should this fire?",
            )
        )
    if draft.action.side is None or draft.action.quantity is None:
        issues.append(
            ValidationIssue(
                slot="action",
                code="missing_slot",
                message="action template incomplete",
                spoken="I still need the action — buy or sell, and a size.",
            )
        )
    if draft.status.value == "armed" and not (draft.rationale_text or "").strip():
        issues.append(
            ValidationIssue(
                slot="rationale_text",
                code="rationale_required",
                message="rationale required before arming",
                spoken="In your words — why does this rule exist? I record that before it arms.",
            )
        )
    return issues


def _spoken_missing(slot: str) -> str:
    return {
        "side": "Buy or sell?",
        "quantity": "What size?",
        "instrument": "Which name?",
        "order_type": "Market or limit?",
        "limit_price": "At what limit?",
        "stop_price": "At what stop?",
    }.get(slot, f"I'm missing {slot.replace('_', ' ')}.")
