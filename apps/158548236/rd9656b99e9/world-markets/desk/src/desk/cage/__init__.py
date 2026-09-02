"""The Cage — deterministic execution. LLM proposes; this module commits."""

from desk.cage.machine import BRAKE_LINE, PARK_LINE, TEACH_ASSENT, Cage, CageResult
from desk.cage.readback import render_readback
from desk.cage.reserved import detect_reserved, is_soft_affirmative
from desk.cage.types import (
    CageState,
    CardPayload,
    MandateDraft,
    MandateStatus,
    MandateTrigger,
    OrderDraft,
    Quantity,
    QuantityKind,
    ResolvedInstrument,
    SpokenText,
)
from desk.cage.validate import validate_mandate, validate_order

__all__ = [
    "BRAKE_LINE",
    "PARK_LINE",
    "TEACH_ASSENT",
    "Cage",
    "CageResult",
    "CageState",
    "CardPayload",
    "MandateDraft",
    "MandateStatus",
    "MandateTrigger",
    "OrderDraft",
    "Quantity",
    "QuantityKind",
    "ResolvedInstrument",
    "SpokenText",
    "detect_reserved",
    "is_soft_affirmative",
    "render_readback",
    "validate_mandate",
    "validate_order",
]
