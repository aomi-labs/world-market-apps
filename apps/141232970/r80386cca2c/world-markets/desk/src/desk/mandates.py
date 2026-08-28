from __future__ import annotations

from datetime import datetime, timedelta, timezone
from decimal import Decimal
from typing import Any, Callable

from desk.cage.machine import Cage
from desk.cage.types import MandateDraft, MandateStatus, OrderDraft
from desk.cage.validate import resolve_base_quantity
from desk.config import DeskConfig


class MandateWatcher:
    def __init__(
        self,
        config: DeskConfig,
        broker: Any,
        cage: Cage,
        *,
        clock: Callable[[], datetime] | None = None,
    ) -> None:
        self.config = config
        self.broker = broker
        self.cage = cage
        self.clock = clock or (lambda: datetime.now(timezone.utc))
        self.armed: dict[str, MandateDraft] = {}
        self.fires: list[dict[str, Any]] = []
        self.queued_reports: list[str] = []
        self.session_live = True

    def register(self, mandate: MandateDraft) -> None:
        self.armed[mandate.id] = mandate

    def suspend(self, mandate_id: str, hours: int) -> None:
        m = self.armed.get(mandate_id)
        if not m:
            return
        m.status = MandateStatus.SUSPENDED
        m.suspended_until = self.clock() + timedelta(hours=hours)

    def revoke(self, mandate_id: str) -> None:
        m = self.armed.get(mandate_id)
        if not m:
            return
        m.status = MandateStatus.REVOKED

    def poll(self, now: datetime | None = None) -> list[str]:
        now = now or self.clock()
        reports: list[str] = []
        for mandate in list(self.armed.values()):
            if mandate.status is MandateStatus.SUSPENDED:
                if mandate.suspended_until and now >= mandate.suspended_until:
                    mandate.status = MandateStatus.ARMED
                    mandate.suspended_until = None
                else:
                    continue
            if mandate.status is not MandateStatus.ARMED:
                continue
            if mandate.expires_at and now >= mandate.expires_at:
                mandate.status = MandateStatus.EXPIRED
                continue
            inst = mandate.trigger.instrument
            if inst is None or mandate.trigger.price is None:
                continue
            mark = self.broker.mark(inst.symbol, inst.product)
            if mark is None:
                continue
            if not _crossed(mark, mandate.trigger.comparator, mandate.trigger.price):
                mandate.window_started_at = None
                continue
            if mandate.window_started_at is None:
                mandate.window_started_at = now
                continue
            elapsed = (now - mandate.window_started_at).total_seconds()
            if elapsed < mandate.confirmation_window_sec:
                continue
            report = self._fire(mandate, mark)
            if report:
                reports.append(report)
        return reports

    def _fire(self, mandate: MandateDraft, mark: Decimal) -> str | None:
        action = mandate.action
        if action.instrument is None:
            action.instrument = mandate.trigger.instrument
        book = self.broker.snapshot()
        pos_qty = Decimal(0)
        inst = action.instrument
        assert inst is not None
        for pos in book.positions:
            if pos.symbol == inst.symbol and pos.product == inst.product:
                pos_qty = pos.quantity
                break
        base_qty, issue = resolve_base_quantity(action, mark=mark, position_qty=pos_qty)
        if issue or base_qty is None:
            return None
        # Same Cage validation path: reuse propose + force submit via broker after validate.
        from desk.cage.validate import validate_order

        issues = validate_order(
            action,
            config=self.config,
            book=book,
            mark=mark,
            instrument_chosen=True,
            policy=self.cage.policy,
        )
        if issues:
            return None
        receipt = self.broker.submit(action, base_qty)
        mandate.status = MandateStatus.FIRED
        self.fires.append({"mandate_id": mandate.id, "receipt": receipt, "mark": str(mark)})
        drop = getattr(self.broker, "drop_watch", None)
        if callable(drop):
            drop(mandate.id)
        remain = "No residual position." if pos_qty == 0 or action.quantity and action.quantity.kind.value == "pct_of_position" and action.quantity.value in {Decimal("1"), Decimal("100")} else "What's left stays on the book."
        name = mandate.name or inst.symbol
        side = action.side or "sell"
        report = (
            f"Your {name} rule just fired — {side} {base_qty} {inst.name} at {mark}. {remain}"
        )
        if self.session_live:
            return report
        self.queued_reports.append(report)
        return None


def _crossed(mark: Decimal, comparator: str, price: Decimal) -> bool:
    return {
        "lt": mark < price,
        "lte": mark <= price,
        "gt": mark > price,
        "gte": mark >= price,
    }.get(comparator, False)
