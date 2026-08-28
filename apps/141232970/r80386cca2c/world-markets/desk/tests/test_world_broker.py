from decimal import Decimal

from desk.cage.types import OrderDraft, Quantity, ResolvedInstrument
from desk.world_broker import WorldBroker


class FakeRails:
    def __init__(self) -> None:
        self.account_id = 17
        self.orders: list[dict] = []
        self.cancelled: list[dict] = []
        self.watches: list[dict] = []
        self.dropped: list[str] = []
        self._ctx = {
            "ok": True,
            "account_id": 17,
            "portfolio": {
                "total_usd_value": "10000.00",
                "dollarpower": {
                    "equivalent_usd": "10000.00",
                    "committed_usd": "8000.00",
                },
                "positions": [
                    {"symbol": "USDT", "quantity": "2000", "usd_value": "2000.00", "asset_type": "spot"},
                    {"symbol": "WETH", "quantity": "2", "usd_value": "7600.00", "asset_type": "spot"},
                ],
            },
            "products": {
                "products": [
                    {
                        "symbol": "WETH",
                        "name": "Wrapped Ether",
                        "product": "spot",
                        "quote_symbol": "USDT",
                        "mark_price": "3800",
                        "base_token_id": 2,
                        "quote_token_id": 1,
                    }
                ]
            },
            "ledger_summary": {"holding": 1, "needs_you": 0},
            "pnl": {"account": {"total": {"value": "12.50"}}},
        }

    def context(self) -> dict:
        return self._ctx

    def place_order(self, body: dict) -> dict:
        self.orders.append(body)
        return {"ok": True, "transaction_hash": "0xabc", "price": "3800"}

    def cancel_order(self, body: dict) -> dict:
        self.cancelled.append(body)
        return {"ok": True}

    def set_watch(self, body: dict) -> dict:
        self.watches.append(body)
        return {"ok": True, "watch": {"id": "w-desk-1"}}

    def cancel_watch(self, watch_id: str) -> dict:
        self.dropped.append(watch_id)
        return {"ok": True}

    def mark_series(self, symbol: str) -> list[dict]:
        assert symbol.upper() == "WETH"
        return [
            {"ts": 1_700_000_000, "mark": "3900"},
            {"ts": 1_700_086_400, "mark": "2900"},
        ]


def test_world_snapshot_and_quote():
    broker = WorldBroker(FakeRails())
    snap = broker.snapshot()
    assert snap.cash == Decimal("2000")
    assert len(snap.positions) == 1
    assert snap.positions[0].symbol == "WETH"
    assert broker.mark("WETH", "spot") == Decimal("3800")
    q = broker.quote("WETH", "spot", "Wrapped Ether")
    assert q is not None and q.source == "world-live"


def test_world_submit_hits_sidecar_shape():
    rails = FakeRails()
    broker = WorldBroker(rails)
    inst = ResolvedInstrument(
        symbol="WETH",
        name="Wrapped Ether",
        product="spot",
        token_id=2,
        confidence=0.99,
    )
    draft = OrderDraft(
        side="buy",
        quantity=Quantity(kind="base", value=Decimal("0.1")),
        instrument=inst,
        order_type="market",
    )
    rec = broker.submit(draft, Decimal("0.1"))
    assert rec["status"] == "filled"
    assert rec["order_id"] == "0xabc"
    assert rails.orders[0]["base_token_id"] == 2
    assert rails.orders[0]["account_id"] == 17
    assert rails.orders[0]["product"] == "spot"


def test_simulate_and_watch_sync():
    rails = FakeRails()
    broker = WorldBroker(rails)
    fires = broker.simulate_trigger("WETH", "lt", Decimal("3000"))
    assert len(fires) == 1
    inst = ResolvedInstrument(symbol="WETH", name="Wrapped Ether", product="spot", confidence=0.99)

    class M:
        id = "m1"
        trigger = type("T", (), {"instrument": inst, "price": Decimal("3000"), "comparator": "lt"})()

    assert broker.register_watch(M()) == "w-desk-1"
    assert "below" in rails.watches[0]["phrase"]
    broker.drop_watch("m1")
    assert rails.dropped == ["w-desk-1"]
    notes = broker.open_notes()
    assert "instruction" in (notes["ledger"] or "")
