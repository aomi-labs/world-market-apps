from decimal import Decimal

from desk.config import DeskConfig, load_config
from desk.instruments import InstrumentResolver
from desk.interrupt import InterruptionTracker
from desk.policy import AomiPolicy
from desk.cage.types import OrderDraft, Quantity, ResolvedInstrument
from desk.cage.validate import resolve_base_quantity, validate_order
from desk.config import DeskConfig as C
from stub_broker import StubBroker


def test_load_config_missing(tmp_path):
    cfg = load_config(tmp_path / "nope.yaml")
    assert cfg.verbosity == "expert"


def test_resolver_spelled_and_fuzzy():
    r = InstrumentResolver()
    spelled = r.search("W-E-T-H")
    assert spelled and spelled[0].instrument.symbol == "WETH"
    fuzzy = r.search("wrapd ether")
    assert fuzzy
    empty = r.search("   ")
    assert empty == []
    terms = r.keyterms(["FOO"])
    assert "WETH" in terms and len(terms) <= 100


def test_interrupt_tracker():
    t = InterruptionTracker()
    t.start("Buy point five ether Done?")
    t.on_marker(4)
    info = t.barge_in()
    assert "heard_text" in info
    t.start("Hello world")
    t.complete()
    assert t.completed
    assert "interrupted" in t.context_note() or t.heard_up_to_word >= 0


def test_policy_absent_and_wrong_market():
    p = AomiPolicy(None)
    inst = ResolvedInstrument(symbol="WBTC", name="BTC", product="spot", confidence=0.99)
    draft = OrderDraft(side="buy", quantity=Quantity(kind="base", value=1), instrument=inst, order_type="market")
    assert p.allows(draft, Decimal("1")).code == "missing_mandate"
    p2 = AomiPolicy.from_path("placeholder")
    assert p2.allows(draft, Decimal("1")).code == "market_not_permitted"
    weth = inst.model_copy(update={"symbol": "WETH", "name": "Wrapped Ether"})
    draft.instrument = weth
    assert p2.allows(draft, Decimal("999999")).code == "max_position_notional"
    assert p2.allows(draft, Decimal("10")) is None
    none = AomiPolicy.from_path("none")
    assert none.absent


def test_resolve_qty_errors():
    d = OrderDraft()
    q, err = resolve_base_quantity(d, mark=None, position_qty=Decimal(0))
    assert err and err.code == "missing_quantity"
    d.quantity = Quantity(kind="base", value=0)
    q, err = resolve_base_quantity(d, mark=Decimal(1), position_qty=Decimal(0))
    assert err.code == "non_positive"
    d.quantity = Quantity(kind="dollars", value=10)
    q, err = resolve_base_quantity(d, mark=None, position_qty=Decimal(0))
    assert err.code == "no_mark"
    d.quantity = Quantity(kind="pct_of_position", value=50)
    q, err = resolve_base_quantity(d, mark=Decimal(1), position_qty=Decimal(0))
    assert err.code == "no_position"
    q, err = resolve_base_quantity(d, mark=Decimal(1), position_qty=Decimal(4))
    assert q == Decimal(2)


def test_validate_without_policy_ok():
    broker = StubBroker()
    inst = ResolvedInstrument(symbol="WETH", name="Wrapped Ether", product="spot", confidence=0.99)
    draft = OrderDraft(
        side="buy",
        quantity=Quantity(kind="base", value=Decimal("0.01")),
        instrument=inst,
        order_type="market",
    )
    issues = validate_order(draft, config=C(), book=broker.snapshot(), mark=Decimal("3800"), instrument_chosen=True)
    assert issues == []
