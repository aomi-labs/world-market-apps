"""World book: marks, snapshot, history, and sidecar submit."""

from __future__ import annotations

from datetime import datetime, timezone
from decimal import Decimal, InvalidOperation
from typing import Any
from uuid import uuid4

from desk.cage.types import OrderDraft, PortfolioSnapshot, Position, Quote
from desk.config import DeskConfig
from desk.instruments import InstrumentRow, default_universe
from desk.rails import WorldRails


def _dec(value: Any) -> Decimal | None:
    if value is None or value == "":
        return None
    try:
        return Decimal(str(value))
    except (InvalidOperation, ValueError):
        return None


class WorldBroker:
    """Broker protocol against this repo's account, brain, and execution sidecar."""

    def __init__(self, rails: WorldRails, *, cache_ttl: float = 2.0) -> None:
        self.rails = rails
        self.cache_ttl = cache_ttl
        self._ctx: dict[str, Any] | None = None
        self._ctx_at = 0.0
        self.working: dict[str, dict[str, Any]] = {}
        self.watch_ids: dict[str, str] = {}
        self._now = lambda: datetime.now(timezone.utc)

    @classmethod
    def from_config(cls, config: DeskConfig) -> WorldBroker:
        rails = WorldRails.from_env(
            account_id=config.world_account_id,
            context_url=config.desk_context_url,
            brain_url=config.world_brain_url,
            execution_url=config.world_execution_url,
            bridge_token=config.desk_bridge_token,
        )
        return cls(rails)

    def invalidate(self) -> None:
        self._ctx = None
        self._ctx_at = 0.0

    def context(self) -> dict[str, Any]:
        import time

        now = time.monotonic()
        if self._ctx is None or now - self._ctx_at > self.cache_ttl:
            self._ctx = self.rails.context()
            self._ctx_at = now
        return self._ctx

    def universe(self) -> list[InstrumentRow]:
        products = ((self.context().get("products") or {}).get("products")) or []
        defaults = {(row.symbol, row.product): row for row in default_universe()}
        rows: list[InstrumentRow] = []
        seen: set[tuple[str, str]] = set()
        for item in products:
            product = str(item.get("product") or "")
            if product not in {"spot", "perp"}:
                continue
            symbol = str(item.get("symbol") or "").upper()
            if not symbol:
                continue
            key = (symbol, product)
            seen.add(key)
            mark = _dec(item.get("mark_price"))
            token_id = item.get("base_token_id")
            token_id_i = int(token_id) if token_id is not None else None
            quote = str(item.get("quote_symbol") or "USDT")
            base = defaults.get(key)
            if base is not None:
                rows.append(
                    InstrumentRow(
                        symbol=base.symbol,
                        name=base.name,
                        product=base.product,
                        quote=quote or base.quote,
                        aliases=list(base.aliases),
                        description=base.description,
                        last_price=mark or base.last_price,
                        adv=base.adv,
                        token_id=token_id_i if token_id_i is not None else base.token_id,
                        confusable_group=base.confusable_group,
                    )
                )
                continue
            name = str(item.get("name") or symbol)
            rows.append(
                InstrumentRow(
                    symbol=symbol,
                    name=name,
                    product=product,
                    quote=quote,
                    aliases=[symbol.lower(), name.lower()],
                    description=f"{name} {product} on World",
                    last_price=mark or Decimal("0"),
                    adv=Decimal("0"),
                    token_id=token_id_i,
                )
            )
        for key, row in defaults.items():
            if key not in seen:
                rows.append(row)
        return rows or default_universe()

    def snapshot(self) -> PortfolioSnapshot:
        ctx = self.context()
        portfolio = ctx.get("portfolio") or {}
        products = {
            (str(p.get("symbol") or "").upper(), str(p.get("product") or "")): p
            for p in ((ctx.get("products") or {}).get("products") or [])
        }
        positions: list[Position] = []
        cash = Decimal("0")
        for row in portfolio.get("positions") or []:
            symbol = str(row.get("symbol") or "").upper()
            kind = str(row.get("asset_type") or "spot")
            qty = _dec(row.get("quantity")) or Decimal("0")
            if qty == 0:
                continue
            side = str(row.get("side") or "").lower()
            if kind == "perp" and side in {"short", "sell"}:
                qty = -qty
            if symbol in {"USDT", "USDC"} and kind == "spot":
                cash += qty
                continue
            product = "perp" if kind == "perp" else "spot" if kind == "spot" else kind
            prod = products.get((symbol, product)) or products.get((symbol, "spot"))
            mark = _dec(row.get("usd_value"))
            if qty and mark is not None:
                px = (mark / qty) if qty != 0 else None
            else:
                px = _dec((prod or {}).get("mark_price"))
            positions.append(
                Position(
                    symbol=symbol,
                    product=product if product in {"spot", "perp"} else "spot",
                    quantity=qty,
                    avg_price=None,
                    mark=px,
                )
            )
        equity = _dec(portfolio.get("total_usd_value"))
        if equity is None:
            dp = portfolio.get("dollarpower") or {}
            equity = _dec(dp.get("equivalent_usd")) or cash
        if cash == 0:
            dp = portfolio.get("dollarpower") or {}
            committed = _dec(dp.get("committed_usd"))
            if equity is not None and committed is not None:
                leftover = equity - committed
                if leftover > 0:
                    cash = leftover
        return PortfolioSnapshot(
            equity=equity or cash,
            cash=cash,
            positions=positions,
            as_of=self._now(),
        )

    def mark(self, symbol: str, product: str) -> Decimal | None:
        key = (symbol.upper(), product)
        for item in ((self.context().get("products") or {}).get("products") or []):
            if (
                str(item.get("symbol") or "").upper() == key[0]
                and str(item.get("product") or "") == key[1]
            ):
                return _dec(item.get("mark_price"))
        return None

    def spread_bps(self, symbol: str, product: str) -> Decimal | None:
        _ = symbol, product
        return None

    def quote(self, symbol: str, product: str, name: str = "") -> Quote | None:
        mark = self.mark(symbol, product)
        if mark is None:
            return None
        return Quote(
            symbol=symbol.upper(),
            name=name or symbol.upper(),
            product=product,
            mark=mark,
            bid=None,
            ask=None,
            as_of=self._now(),
            source="world-live",
        )

    def token_ids(self, symbol: str, product: str) -> tuple[int | None, int | None]:
        for item in ((self.context().get("products") or {}).get("products") or []):
            if (
                str(item.get("symbol") or "").upper() == symbol.upper()
                and str(item.get("product") or "") == product
            ):
                base = item.get("base_token_id")
                quote = item.get("quote_token_id")
                return (
                    int(base) if base is not None else None,
                    int(quote) if quote is not None else None,
                )
        return None, None

    def submit(self, draft: OrderDraft, base_qty: Decimal) -> dict[str, Any]:
        assert draft.instrument is not None and draft.side is not None
        inst = draft.instrument
        base_id, quote_id = self.token_ids(inst.symbol, inst.product)
        if inst.token_id is not None:
            base_id = inst.token_id
        if base_id is None:
            raise RuntimeError(f"no token id for {inst.symbol} {inst.product}")
        order_type = "limit" if draft.order_type == "limit" else "market"
        body: dict[str, Any] = {
            "account_id": self.rails.account_id,
            "product": inst.product,
            "side": draft.side,
            "base_token_id": base_id,
            "quantity": str(base_qty),
            "order_type": order_type,
        }
        if inst.product != "lend":
            body["quote_token_id"] = quote_id if quote_id is not None else 1
        if draft.limit_price is not None:
            body["price"] = str(draft.limit_price)
        receipt = self.rails.place_order(body)
        self.invalidate()
        order_id = str(
            receipt.get("order_id")
            or receipt.get("transaction_hash")
            or uuid4().hex[:10]
        )
        fill_price = receipt.get("price") or receipt.get("fill_price")
        rec = {
            "order_id": order_id,
            "status": "filled" if receipt.get("ok", True) else "working",
            "fill_price": str(fill_price) if fill_price is not None else None,
            "quantity": str(base_qty),
            "side": draft.side,
            "symbol": inst.symbol,
            "product": inst.product,
            "draft_id": draft.id,
            "receipt": receipt,
        }
        if rec["status"] == "working":
            self.working[order_id] = rec
        return rec

    def cancel_in_flight(self, order_id: str) -> bool:
        rec = self.working.pop(order_id, None)
        if rec is None:
            return False
        base_id, quote_id = self.token_ids(rec["symbol"], rec["product"])
        body: dict[str, Any] = {
            "account_id": self.rails.account_id,
            "product": rec["product"],
            "side": rec["side"],
            "base_token_id": base_id,
            "order_id": order_id,
        }
        if rec["product"] != "lend":
            body["quote_token_id"] = quote_id if quote_id is not None else 1
        try:
            self.rails.cancel_order(body)
        except RuntimeError:
            return False
        self.invalidate()
        return True

    def simulate_trigger(
        self, symbol: str, comparator: str, price: Decimal
    ) -> list[datetime]:
        fires: list[datetime] = []
        try:
            series = self.rails.mark_series(symbol)
        except RuntimeError:
            series = []
        by_day: dict[datetime, Decimal] = {}
        for point in series:
            ts = point.get("ts")
            mark = _dec(point.get("mark"))
            if ts is None or mark is None:
                continue
            day = datetime.fromtimestamp(int(ts), tz=timezone.utc).replace(
                hour=0, minute=0, second=0, microsecond=0
            )
            by_day[day] = mark
        for day, px in sorted(by_day.items()):
            hit = {
                "lt": px < price,
                "lte": px <= price,
                "gt": px > price,
                "gte": px >= price,
            }.get(comparator, False)
            if hit:
                fires.append(day)
        return fires

    def register_watch(self, mandate: Any) -> str | None:
        trigger = getattr(mandate, "trigger", None)
        inst = getattr(trigger, "instrument", None) if trigger else None
        price = getattr(trigger, "price", None) if trigger else None
        if inst is None or price is None:
            return None
        cmp_word = {
            "lt": "drops below",
            "lte": "drops below",
            "gt": "rises above",
            "gte": "rises above",
        }.get(str(getattr(trigger, "comparator", "lt")), "drops below")
        phrase = f"tell me if {inst.symbol} {cmp_word} {price}"
        try:
            out = self.rails.set_watch(
                {
                    "phrase": phrase,
                    "symbol": inst.symbol,
                    "token_id": inst.token_id,
                    "fire_mode": "once",
                }
            )
        except RuntimeError:
            return None
        watch = out.get("watch") or {}
        watch_id = watch.get("id")
        if watch_id:
            self.watch_ids[str(getattr(mandate, "id", ""))] = str(watch_id)
        return str(watch_id) if watch_id else None

    def drop_watch(self, mandate_id: str) -> None:
        watch_id = self.watch_ids.pop(mandate_id, None)
        if not watch_id:
            return
        try:
            self.rails.cancel_watch(watch_id)
        except RuntimeError:
            return

    def open_notes(self) -> dict[str, Any]:
        ctx = self.context()
        summary = ctx.get("ledger_summary") or {}
        holding = summary.get("holding")
        needs = summary.get("needs_you")
        ledger_line = None
        if holding:
            ledger_line = f"{holding} instruction(s) on the ledger"
            if needs:
                ledger_line += f", {needs} need you in the thread"
            ledger_line += "."
        pnl = ctx.get("pnl") or {}
        account = pnl.get("account") or {}
        total = ((account.get("total") or {}).get("value")) if isinstance(account, dict) else None
        pnl_line = None
        # Open liturgy forbids day P&L phrasing; lifetime mark-vs-entry is allowed as context.
        if total not in (None, "", "0"):
            pnl_line = f"Perp lifetime mark versus entry is {total} USDT."
        return {
            "ledger": ledger_line,
            "pnl": pnl_line,
        }
