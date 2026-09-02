from __future__ import annotations

from desk.cage.types import CageState, OrderDraft

MANDATORY_ORDER_SLOTS = ("side", "quantity", "instrument", "order_type")


def missing_order_slots(draft: OrderDraft) -> list[str]:
    missing: list[str] = []
    if draft.side is None:
        missing.append("side")
    if draft.quantity is None:
        missing.append("quantity")
    if draft.instrument is None:
        missing.append("instrument")
    if draft.order_type is None:
        missing.append("order_type")
    if draft.order_type in {"limit", "stop_limit"} and draft.limit_price is None:
        missing.append("limit_price")
    if draft.order_type in {"stop", "stop_limit"} and draft.stop_price is None:
        missing.append("stop_price")
    return missing


def instrument_ready(draft: OrderDraft, threshold: float, chosen: bool) -> bool:
    inst = draft.instrument
    if inst is None:
        return False
    return inst.confidence >= threshold or chosen


# States in which a reserved word is "matching" (spec §3.3).
RESERVED_ACTIVE: dict[str, frozenset[CageState]] = {
    "done": frozenset({CageState.ARMED_FOR_ASSENT, CageState.READBACK}),
    "off": frozenset({CageState.ARMED_FOR_ASSENT, CageState.READBACK, CageState.ASSEMBLING}),
    "hold": frozenset({CageState.ARMED_FOR_ASSENT, CageState.READBACK}),
    "cancel": frozenset(CageState),
    "stop": frozenset(CageState),
    "repeat": frozenset(
        {
            CageState.READBACK,
            CageState.ARMED_FOR_ASSENT,
            CageState.ASSEMBLING,
            CageState.PARKED,
        }
    ),
    "slower": frozenset(CageState),
    "show me": frozenset(
        {
            CageState.ASSEMBLING,
            CageState.READBACK,
            CageState.ARMED_FOR_ASSENT,
            CageState.PARKED,
            CageState.SUBMITTED,
            CageState.WORKING,
            CageState.FILLED,
        }
    ),
}
