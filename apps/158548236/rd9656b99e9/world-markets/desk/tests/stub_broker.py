"""In-memory World-book stand-in for Cage tests. Production always uses WorldBroker."""

from __future__ import annotations

from datetime import datetime, timedelta, timezone
from decimal import Decimal
from typing import Any
from uuid import uuid4

from desk.cage.types import OrderDraft, PortfolioSnapshot, Position, Quote


class StubBroker:
    def __init__(
        self,
        *,
        equity: Decimal = Decimal("100000"),
        marks: dict[tuple[str, str], Decimal] | None = None,
        immediate_fills: bool = True,
    ) -> None:
        self.cash = Decimal(equity)
        self.starting_equity = Decimal(equity)
        self.positions: dict[tuple[str, str], Position] = {}
        self.marks = marks or {
            ("WETH", "spot"): Decimal("3800"),
            ("WETH", "perp"): Decimal("3800"),
            ("WBTC", "spot"): Decimal("95000"),
            ("WBTC", "perp"): Decimal("95000"),
            ("USDT", "spot"): Decimal("1"),
        }
        self.spreads_bps: dict[tuple[str, str], Decimal] = {}
        self.immediate_fills = immediate_fills
        self.working: dict[str, dict] = {}
        self.fills: list[dict] = []
        self.daily_bars: dict[str, list[tuple[datetime, Decimal]]] = {}
        self._seed_bars()

    def _seed_bars(self) -> None:
        now = datetime.now(timezone.utc).replace(hour=0, minute=0, second=0, microsecond=0)
        for (sym, _prod), mark in list(self.marks.items()):
            if sym == "USDT":
                continue
            bars = []
            for i in range(30, 0, -1):
                day = now - timedelta(days=i)
                px = mark
                if sym == "WETH":
                    px = Decimal("3600") + Decimal(i * 15)
                    if i < 10:
                        px = Decimal("2900") + Decimal(i * 20)
                bars.append((day, px))
            self.daily_bars[sym] = bars

    def set_mark(self, symbol: str, product: str, price: Decimal) -> None:
        self.marks[(symbol.upper(), product)] = Decimal(price)

    def snapshot(self) -> PortfolioSnapshot:
        pos = list(self.positions.values())
        mtm = Decimal(0)
        for p in pos:
            mark = self.mark(p.symbol, p.product) or Decimal(0)
            mtm += p.quantity * mark
        return PortfolioSnapshot(equity=self.cash + mtm, cash=self.cash, positions=pos)

    def mark(self, symbol: str, product: str) -> Decimal | None:
        return self.marks.get((symbol.upper(), product))

    def spread_bps(self, symbol: str, product: str) -> Decimal | None:
        return self.spreads_bps.get((symbol.upper(), product))

    def quote(self, symbol: str, product: str, name: str = "") -> Quote | None:
        mark = self.mark(symbol, product)
        if mark is None:
            return None
        spread = self.spread_bps(symbol, product) or Decimal("5")
        half = mark * spread / Decimal(10_000) / Decimal(2)
        return Quote(
            symbol=symbol.upper(),
            name=name or symbol.upper(),
            product=product,
            mark=mark,
            bid=mark - half,
            ask=mark + half,
            as_of=datetime.now(timezone.utc),
            source="stub",
        )

    def submit(self, draft: OrderDraft, base_qty: Decimal) -> dict:
        assert draft.instrument is not None and draft.side is not None
        order_id = uuid4().hex[:10]
        inst = draft.instrument
        mark = self.mark(inst.symbol, inst.product) or Decimal(0)
        fill_price = mark
        if draft.order_type == "limit" and draft.limit_price is not None:
            fill_price = draft.limit_price
        status = "filled" if self.immediate_fills else "working"
        rec = {
            "order_id": order_id,
            "status": status,
            "fill_price": str(fill_price),
            "quantity": str(base_qty),
            "side": draft.side,
            "symbol": inst.symbol,
            "product": inst.product,
            "draft_id": draft.id,
        }
        if status == "working":
            self.working[order_id] = rec
            return rec
        self._apply_fill(draft.side, inst.symbol, inst.product, base_qty, fill_price)
        self.fills.append(rec)
        return rec

    def _apply_fill(
        self, side: str, symbol: str, product: str, qty: Decimal, price: Decimal
    ) -> None:
        key = (symbol, product)
        signed = qty if side == "buy" else -qty
        self.cash -= signed * price
        existing = self.positions.get(key)
        if existing is None:
            self.positions[key] = Position(
                symbol=symbol, product=product, quantity=signed, avg_price=price, mark=price
            )
        else:
            existing.quantity += signed
            existing.mark = price
            if existing.quantity == 0:
                del self.positions[key]

    def cancel_in_flight(self, order_id: str) -> bool:
        return self.working.pop(order_id, None) is not None

    def simulate_trigger(
        self, symbol: str, comparator: str, price: Decimal
    ) -> list[datetime]:
        fires: list[datetime] = []
        for day, px in self.daily_bars.get(symbol, []):
            hit = {
                "lt": px < price,
                "lte": px <= price,
                "gt": px > price,
                "gte": px >= price,
            }.get(comparator, False)
            if hit:
                fires.append(day)
        return fires

    def universe(self):
        return None

    def register_watch(self, mandate: Any) -> str | None:
        _ = mandate
        return None

    def drop_watch(self, mandate_id: str) -> None:
        _ = mandate_id

    def open_notes(self) -> dict[str, Any]:
        return {"ledger": None, "pnl": None}
