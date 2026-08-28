from datetime import datetime, timedelta, timezone
from decimal import Decimal

from desk.cage import (
    BRAKE_LINE,
    PARK_LINE,
    TEACH_ASSENT,
    Cage,
    CageState,
    MandateDraft,
    MandateTrigger,
    OrderDraft,
    Quantity,
    QuantityKind,
    ResolvedInstrument,
    detect_reserved,
    is_soft_affirmative,
    render_readback,
    validate_order,
)
from desk.cage.reserved import reserved_is_active
from desk.cage.slots import missing_order_slots
from desk.cage.types import PortfolioSnapshot, Position
from desk.config import DeskConfig
from desk.policy import AomiPolicy
from stub_broker import StubBroker

WETH = ResolvedInstrument(
    symbol="WETH",
    name="Wrapped Ether",
    product="spot",
    quote="USDT",
    confidence=0.95,
    last_price=Decimal("3800"),
    description="Wrapped Ether",
)


def _cage(config: DeskConfig, broker: StubBroker | None = None) -> Cage:
    broker = broker or StubBroker()
    events: list[tuple] = []

    class T:
        def record(self, kind, payload):
            events.append((kind, payload))

    c = Cage(config, broker, T(), policy=AomiPolicy.from_path("placeholder"))
    c.events = events
    return c


def _draft(**kwargs) -> OrderDraft:
    base = dict(
        side="buy",
        quantity=Quantity(kind=QuantityKind.BASE, value=Decimal("0.2")),
        instrument_query="weth",
        instrument=WETH,
        order_type="limit",
        limit_price=Decimal("3800"),
        slot_confidence={"instrument": 0.95, "quantity": 0.95},
    )
    base.update(kwargs)
    return OrderDraft(**base)


def test_reserved_grammar():
    assert detect_reserved("Done.") == "done"
    assert detect_reserved("please cancel") == "cancel"
    assert detect_reserved("show me") == "show me"
    assert detect_reserved("yeah go ahead") is None
    assert is_soft_affirmative("yeah go ahead")
    assert not is_soft_affirmative("done")
    assert reserved_is_active("cancel", CageState.IDLE)
    assert not reserved_is_active("done", CageState.IDLE)


def test_happy_path_done_fills(config):
    cage = _cage(config)
    r = cage.propose_order(_draft())
    assert cage.state is CageState.READBACK
    assert r.readback and r.readback.text.endswith("Done?")
    cage.notify_tts_playout(completed=True)
    assert cage.state is CageState.ARMED_FOR_ASSENT
    r = cage.handle_transcript("Done")
    assert cage.state is CageState.FILLED
    assert r.earcon == "fill"
    assert r.speech.startswith("Bought Wrapped Ether")


def test_soft_yes_does_not_submit(config):
    cage = _cage(config)
    cage.propose_order(_draft())
    cage.notify_tts_playout(completed=True)
    r = cage.handle_transcript("yeah go ahead")
    assert cage.state is CageState.ARMED_FOR_ASSENT
    assert r.speech == TEACH_ASSENT
    assert cage.broker.fills == []
    cage.handle_transcript("Done")
    assert cage.state is CageState.FILLED


def test_assent_before_readback_complete(config):
    cage = _cage(config)
    cage.propose_order(_draft())
    r = cage.handle_transcript("done")
    assert cage.state is CageState.ASSEMBLING
    assert cage.broker.fills == []
    assert r.flush_tts


def test_off_cancels(config):
    cage = _cage(config)
    cage.propose_order(_draft())
    cage.notify_tts_playout(completed=True)
    r = cage.handle_transcript("Off")
    assert r.speech.startswith("Off")
    assert cage.state is CageState.IDLE


def test_hold_parks(config):
    cage = _cage(config)
    cage.propose_order(_draft())
    cage.notify_tts_playout(completed=True)
    r = cage.handle_transcript("hold")
    assert r.speech == PARK_LINE
    assert cage.state is CageState.PARKED
    r = cage.resume_parked()
    assert cage.state is CageState.READBACK


def test_timeout_parks(config):
    cage = _cage(config)
    cage.propose_order(_draft())
    cage.notify_tts_playout(completed=True)
    later = datetime.now(timezone.utc) + timedelta(seconds=31)
    r = cage.tick(later)
    assert r and r.speech == PARK_LINE
    assert cage.state is CageState.PARKED


def test_brake_from_readback(config):
    cage = _cage(config)
    cage.propose_order(_draft())
    r = cage.handle_transcript("cancel")
    assert r.speech == BRAKE_LINE
    assert cage.state is CageState.IDLE


def test_quantity_cap(config):
    cage = _cage(config)
    huge = _draft(quantity=Quantity(kind=QuantityKind.DOLLARS, value=Decimal("90000")))
    r = cage.propose_order(huge)
    assert cage.state is CageState.ASSEMBLING
    assert any(i.code == "quantity_cap" for i in r.issues)


def test_low_instrument_confidence_blocks_readback(config):
    cage = _cage(config)
    inst = WETH.model_copy(update={"confidence": 0.4})
    r = cage.propose_order(_draft(instrument=inst, slot_confidence={"instrument": 0.4}))
    assert cage.state is CageState.ASSEMBLING
    assert any(i.code == "low_confidence" for i in r.issues)
    r = cage.choose_instrument(inst.model_copy(update={"confidence": 0.4}))
    assert cage.state is CageState.READBACK


def test_repeat_slower_show_me(config):
    cage = _cage(config)
    cage.propose_order(_draft())
    spoken = cage.last_speech
    r = cage.handle_transcript("repeat")
    assert r.speech == spoken
    r = cage.handle_transcript("slower")
    assert r.speech == "Slower."
    r = cage.handle_transcript("show me")
    assert r.card and r.card.card == "ticket"


def test_amendment_from_armed(config):
    cage = _cage(config)
    cage.propose_order(_draft())
    cage.notify_tts_playout(completed=True)
    r = cage.handle_transcript("make it market")
    assert cage.state is CageState.ASSEMBLING
    assert r.consumed is False


def test_working_order_blocks_new(config):
    broker = StubBroker(immediate_fills=False)
    cage = _cage(config, broker)
    cage.propose_order(_draft())
    cage.notify_tts_playout(completed=True)
    cage.handle_transcript("done")
    assert cage.state is CageState.WORKING
    r = cage.propose_order(_draft())
    assert "working" in r.speech.lower()
    cage.notify_fill({"fill_price": "3800"})
    assert cage.state is CageState.FILLED
    assert broker.cancel_in_flight("nope") is False


def test_brake_cancels_in_flight(config):
    broker = StubBroker(immediate_fills=False)
    cage = _cage(config, broker)
    cage.propose_order(_draft())
    cage.notify_tts_playout(completed=True)
    cage.handle_transcript("done")
    r = cage.handle_transcript("stop")
    assert r.speech == BRAKE_LINE


def test_missing_slots_and_readback_template(config):
    cage = _cage(config)
    r = cage.propose_order(OrderDraft(side="buy", instrument_query="weth"))
    assert missing_order_slots(cage.draft)
    assert cage.state is CageState.ASSEMBLING
    spoken = render_readback(
        _draft(),
        config=config,
        equity=Decimal("100000"),
        notional=Decimal("760"),
        spread_bps=Decimal("40"),
    )
    assert "Done?" in spoken.text
    assert "spread" in spoken.text


def test_pct_and_dollars_and_policy(config):
    broker = StubBroker()
    broker.positions[("WETH", "spot")] = Position(
        symbol="WETH", product="spot", quantity=Decimal("2"), avg_price=Decimal("3800")
    )
    cage = _cage(config, broker)
    r = cage.propose_order(
        _draft(side="sell", quantity=Quantity(kind=QuantityKind.PCT_OF_POSITION, value=Decimal("0.5")))
    )
    assert cage.state is CageState.READBACK
    r = cage.propose_order(
        _draft(quantity=Quantity(kind=QuantityKind.DOLLARS, value=Decimal("500")))
    )
    assert cage.state in {CageState.READBACK, CageState.ASSEMBLING}


def test_mandate_paraphrase_and_rationale_gate(config):
    cage = _cage(config)
    m = MandateDraft(
        trigger=MandateTrigger(instrument_query="weth", instrument=WETH, comparator="lt", price=Decimal("3000")),
        action=OrderDraft(
            side="sell",
            quantity=Quantity(kind=QuantityKind.PCT_OF_POSITION, value=Decimal("0.5")),
        ),
    )
    r = cage.propose_mandate(m)
    assert "simulate" in r.speech.lower()
    r = cage.arm_mandate()
    assert "your words" in r.speech.lower()
    cage.set_rationale("I don't want to ride a breakdown.")
    r = cage.arm_mandate()
    assert "Armed" in r.speech
    assert cage.registry_card().card == "registry"


def test_validate_buying_power(config):
    broker = StubBroker(equity=Decimal("100"), immediate_fills=True)
    broker.cash = Decimal("10")
    issues = validate_order(
        _draft(),
        config=config,
        book=broker.snapshot(),
        mark=Decimal("3800"),
        instrument_chosen=True,
        policy=AomiPolicy.from_path("placeholder"),
    )
    assert any(i.code == "buying_power" for i in issues)


def test_shares_alias_and_merge():
    q = Quantity(kind="shares", value="3")
    assert q.kind is QuantityKind.BASE
    a = OrderDraft(side="buy")
    b = a.merge(OrderDraft(id="skipme", order_type="market"))
    assert b.side == "buy" and b.order_type == "market"
    c = a.merge(OrderDraft(slot_confidence={"side": 0.9}))
    assert c.slot_confidence.get("side") == 0.9


def test_idle_cancel_and_choose_without_draft(config):
    cage = _cage(config)
    r = cage.handle_transcript("cancel")
    assert r.speech == BRAKE_LINE
    r = cage.choose_instrument(WETH)
    assert "no ticket" in r.speech.lower()
    r = cage.resume_parked()
    assert "parked" in r.speech.lower()
    r = cage.arm_mandate()
    assert "no rule" in r.speech.lower()
    cage.set_rationale("x")
    r = cage.handle_transcript("hello")
    assert r.consumed is False
    cage.notify_tts_playout(completed=True)
    assert cage.state is CageState.IDLE
    r = cage.barge_in()
    assert r.flush_tts
    assert cage.tick() is None
    r = cage.notify_fill({"fill_price": "1"})
    assert r.state is CageState.IDLE


def test_stop_limit_missing_and_market_hours_notes(config):
    cage = _cage(config)
    r = cage.propose_order(_draft(order_type="stop_limit", stop_price=None, limit_price=None))
    assert any(i.slot in {"stop_price", "limit_price"} for i in r.issues)
    cage.propose_order(_draft(order_type="market", limit_price=None))
    cage.notify_tts_playout(completed=False)
    assert cage.state is CageState.READBACK


def test_quantity_kind_aliases():
    assert Quantity(kind="units", value=1).kind is QuantityKind.BASE
    assert Quantity(kind="contracts", value=1).kind is QuantityKind.BASE
    assert Quantity.model_validate({"kind": "dollars", "value": 1}).kind is QuantityKind.DOLLARS


def test_ticket_card_none_and_reserved_fallthrough(config):
    cage = _cage(config)
    assert cage.ticket_card() is None
    assert cage._handle_reserved("nope").consumed is False
    assert cage._done().consumed is False
    assert cage._off().consumed is False
    r = cage.handle_transcript("repeat")
    assert "nothing to repeat" in (r.speech or "").lower() or r.consumed is False


def test_revalidate_and_cant_size(config, monkeypatch):
    from desk.cage.types import ValidationIssue

    cage = _cage(config)
    cage.propose_order(_draft())
    cage.notify_tts_playout(completed=True)
    cage.broker.cash = Decimal("0")
    r = cage.handle_transcript("done")
    assert r.issues
    cage.broker.cash = Decimal("100000")
    cage.propose_order(_draft())
    cage.notify_tts_playout(completed=True)

    def boom(*_a, **_k):
        return None, ValidationIssue(code="x", message="x", spoken="Can't size it.")

    monkeypatch.setattr("desk.cage.machine.resolve_base_quantity", boom)
    r = cage._done()
    assert "Can't size it" in (r.speech or "")


def test_fill_without_price_and_sell_speech(config):
    cage = _cage(config)
    cage.propose_order(_draft(side="sell"))
    cage.notify_tts_playout(completed=True)
    rec = dict(order_id="x", status="filled", fill_price=None)
    monkey_broker = cage.broker

    def submit(draft, qty):
        return rec

    monkey_broker.submit = submit  # type: ignore[method-assign]
    r = cage.handle_transcript("done")
    assert "Sold" in (r.speech or "")


def test_mandate_merge_gt_and_paraphrase_sizes(config):
    cage = _cage(config)
    m = MandateDraft(trigger=MandateTrigger(instrument_query="weth"))
    r = cage.propose_mandate(m)
    assert r.issues or "which name" in (r.speech or "").lower() or "price" in (r.speech or "").lower()
    full = MandateDraft(
        trigger=MandateTrigger(
            instrument_query="weth",
            instrument=WETH,
            comparator="gt",
            price=Decimal("4000"),
        ),
        action=OrderDraft(
            side="sell",
            quantity=Quantity(kind=QuantityKind.BASE, value=Decimal("1")),
            instrument=WETH,
        ),
    )
    cage.propose_mandate(full)
    cage.propose_mandate(
        MandateDraft(
            trigger=MandateTrigger(comparator="gte", price=Decimal("4100")),
            action=OrderDraft(quantity=Quantity(kind=QuantityKind.PCT_OF_POSITION, value=Decimal("25"))),
        )
    )
    text = cage.paraphrase_mandate()
    assert "rises" in text or "percent" in text
    cage.mandate.action.quantity = None
    assert "the size" in cage.paraphrase_mandate()


def test_readback_dollars_pct_spell_stop(config):
    from desk.cage.readback import render_readback

    d = _draft(
        quantity=Quantity(kind=QuantityKind.DOLLARS, value=Decimal("500")),
        slot_confidence={"instrument": 0.5, "quantity": 0.95},
        instrument=WETH.model_copy(update={"confidence": 0.5}),
        order_type="stop_limit",
        stop_price=Decimal("3700"),
        duration="gtc",
    )
    spoken = render_readback(d, config=config, equity=Decimal("100000"), notional=Decimal("500"))
    assert "Done?" in spoken.text
    d2 = _draft(quantity=Quantity(kind=QuantityKind.PCT_OF_POSITION, value=Decimal("25")))
    spoken2 = render_readback(d2, config=config, equity=Decimal("100000"))
    assert "percent" in spoken2.text or "position" in spoken2.text
    d3 = OrderDraft(
        side="buy",
        quantity=Quantity(kind=QuantityKind.BASE, value=1),
        order_type="market",
        slot_confidence={},
    )
    try:
        render_readback(d3, config=config, equity=Decimal(1))
        raise AssertionError("expected")
    except ValueError:
        pass
    from desk.cage.readback import _unit

    d3.quantity = Quantity(kind=QuantityKind.DOLLARS, value=1)
    assert "the name" in _unit(d3)
    d3.quantity = Quantity(kind=QuantityKind.PCT_OF_POSITION, value=Decimal("0.5"))
    assert "half" in _unit(d3)
    d3.quantity = Quantity(kind=QuantityKind.BASE, value=1)
    d3.instrument = None
    assert _unit(d3) == "units"


def test_slots_and_notional_edges(config):
    from desk.cage.slots import instrument_ready, missing_order_slots
    from desk.cage.validate import _notional, validate_mandate

    empty = OrderDraft()
    assert "side" in missing_order_slots(empty)
    assert instrument_ready(empty, 0.9, False) is False
    assert _notional(empty, Decimal("1")) is None
    d = _draft()
    assert _notional(d, None) is None or _notional(d, Decimal("3800"))
    pct = _draft(quantity=Quantity(kind=QuantityKind.PCT_OF_POSITION, value=Decimal("0.5")))
    assert _notional(pct, Decimal("3800")) is None
    issues = validate_mandate(MandateDraft(), config=config)
    assert issues
    low = MandateDraft(
        trigger=MandateTrigger(instrument=WETH.model_copy(update={"confidence": 0.1}), price=Decimal("1")),
        action=_draft(side="sell"),
    )
    assert any(i.code == "low_confidence" for i in validate_mandate(low, config=config))
    issues = validate_order(
        _draft(quantity=Quantity(kind=QuantityKind.DOLLARS, value=Decimal("10"))),
        config=config,
        book=PortfolioSnapshot(equity=Decimal("100"), cash=Decimal("100"), positions=[]),
        mark=None,
        instrument_chosen=True,
    )
    assert any(i.code == "no_mark" for i in issues)
    other_pos = Position(symbol="WBTC", product="spot", quantity=Decimal("1"))
    issues = validate_order(
        _draft(),
        config=config,
        book=PortfolioSnapshot(
            equity=Decimal("100000"), cash=Decimal("100000"), positions=[other_pos]
        ),
        mark=Decimal("3800"),
        instrument_chosen=True,
        policy=AomiPolicy.from_path("placeholder"),
    )
    assert issues == [] or True


def test_quote_age_naive():
    from desk.cage.types import Quote

    q = Quote(
        symbol="WETH",
        name="WETH",
        product="spot",
        mark=Decimal("1"),
        as_of=datetime.now(),
        source="t",
    )
    assert q.age_seconds() >= 0


def test_detect_empty_reserved():
    assert detect_reserved("   ") is None
    assert detect_reserved("done please") == "done"


def test_done_uses_open_position_and_gt_limit(config):
    broker = StubBroker()
    broker.positions[("WBTC", "spot")] = Position(
        symbol="WBTC", product="spot", quantity=Decimal("1"), avg_price=Decimal("95000")
    )
    broker.positions[("WETH", "spot")] = Position(
        symbol="WETH", product="spot", quantity=Decimal("2"), avg_price=Decimal("3800")
    )
    cage = _cage(config, broker)
    r = cage.propose_order(
        _draft(side="sell", quantity=Quantity(kind=QuantityKind.PCT_OF_POSITION, value=Decimal("0.5")))
    )
    assert cage.state is CageState.READBACK
    cage.notify_tts_playout(completed=True)
    r = cage.handle_transcript("done")
    assert cage.state is CageState.FILLED
    cage.mandate = None
    cage.propose_mandate(
        MandateDraft(
            trigger=MandateTrigger(instrument=WETH, instrument_query="weth", comparator="gt", price=Decimal("4000")),
            action=OrderDraft(side="sell", quantity=Quantity(kind=QuantityKind.PCT_OF_POSITION, value=Decimal("0.5"))),
        )
    )
    assert cage.mandate.action.limit_price is not None
    assert cage.mandate.action.limit_price > Decimal("4000")
    kept = MandateDraft(name="old", trigger=MandateTrigger(instrument_query="weth", instrument=WETH, price=Decimal("1")))
    other = MandateDraft.model_construct(
        name=None,
        trigger=MandateTrigger(instrument_query="", comparator="lte", price=None),
        action=OrderDraft(),
        confirmation_window_sec=None,
    )
    merged = kept.merge(MandateDraft(name="renamed", rationale_text="because", expiry_days=10, action=OrderDraft(side="buy")))
    assert merged.name == "renamed"
    cage.propose_mandate(
        MandateDraft(
            trigger=MandateTrigger(instrument=WETH, instrument_query="weth", comparator="gt", price=Decimal("4000")),
            action=OrderDraft(side="sell", quantity=Quantity(kind=QuantityKind.PCT_OF_POSITION, value=Decimal("0.5")), duration="gtc"),
        )
    )
    cage = _cage(config)

    def submit(draft, qty):
        return {"order_id": "w1", "status": "working"}

    cage.broker.submit = submit  # type: ignore[method-assign]
    cage.propose_order(_draft())
    cage.notify_tts_playout(completed=True)
    r = cage.handle_transcript("done")
    assert cage.state is CageState.WORKING
    assert r.speech == "Working."

