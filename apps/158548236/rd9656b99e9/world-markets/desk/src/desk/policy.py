from __future__ import annotations

from decimal import Decimal
from typing import Any

from desk.cage.types import OrderDraft, ResolvedInstrument, ValidationIssue


class AomiPolicy:
    """Local evaluation of the Aomi policy JSON. Desk trigger-mandates are a different object."""

    def __init__(self, doc: dict[str, Any] | None) -> None:
        self.doc = doc or {}
        self.absent = not bool(doc)

    @classmethod
    def from_path(cls, path: str, *, repo_root: str | None = None) -> AomiPolicy:
        from pathlib import Path
        import json

        raw = (path or "placeholder").strip().lower()
        if raw in {"none", "off"}:
            return cls(None)
        if raw in {"", "placeholder", "dev", "-"}:
            candidate = Path(repo_root or Path(__file__).resolve().parents[3]) / "mandate.dev.example.json"
            if not candidate.exists():
                candidate = Path(__file__).resolve().parents[2] / "mandate.dev.example.json"
            if candidate.exists():
                return cls(json.loads(candidate.read_text()))
            return cls(
                {
                    "version": 1,
                    "markets": [
                        {"product": "spot", "base": "WETH", "quote": "USDT"},
                        {"product": "perp", "base": "WETH", "quote": "USDT"},
                    ],
                    "max_position_notional": {"amount": "25000", "quote": "USDT"},
                }
            )
        p = Path(path)
        if not p.exists():
            return cls(None)
        return cls(json.loads(p.read_text()))

    def allows(self, draft: OrderDraft, notional: Decimal) -> ValidationIssue | None:
        if self.absent:
            return ValidationIssue(
                slot=None,
                code="missing_mandate",
                message="No mandate is bound to this account.",
                spoken="No mandate is bound to this account.",
            )
        inst = draft.instrument
        if inst is None:
            return ValidationIssue(
                slot="instrument",
                code="missing_slot",
                message="instrument required",
                spoken="Which name?",
            )
        allowed = False
        for market in self.doc.get("markets") or []:
            if (
                str(market.get("product", "")).lower() == inst.product
                and str(market.get("base", "")).upper() == inst.symbol
                and str(market.get("quote", "USDT")).upper() == inst.quote.upper()
            ):
                allowed = True
                break
        if not allowed:
            return ValidationIssue(
                slot="instrument",
                code="market_not_permitted",
                message=f"{inst.product} {inst.symbol}/{inst.quote} is not on the bound mandate",
                spoken=f"{inst.name} isn't on the bound mandate.",
            )
        cap_raw = (self.doc.get("max_position_notional") or {}).get("amount")
        if cap_raw is not None:
            cap = Decimal(str(cap_raw))
            if notional > cap:
                return ValidationIssue(
                    slot="quantity",
                    code="max_position_notional",
                    message=f"notional {notional} exceeds mandate cap {cap}",
                    spoken="That's above the mandate notional cap.",
                )
        return None
