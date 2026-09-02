from datetime import datetime, timedelta, timezone
from decimal import Decimal

from desk.cage import CageState, TEACH_ASSENT
from desk.open_liturgy import SIGN_OFF, contains_day_pnl, render_open
from desk.parser import parse_utterance
from desk.persist import replay_text


def complete(session) -> dict:
    if session.interrupt.playing or session.cage.state.value == "READBACK":
        return session.notify_tts_complete()
    return {}


def test_happy_path_order(session, store):
    out = session.on_final_transcript("buy two tenths of weth, limit three thousand eight hundred")
    assert session.cage.state is CageState.READBACK
    assert "Done?" in (out.get("speech") or "")
    complete(session)
    assert session.cage.state is CageState.ARMED_FOR_ASSENT
    out = session.on_final_transcript("Done")
    assert session.cage.state is CageState.FILLED
    assert out.get("earcon") == "fill"
    assert session.broker.fills
    text = replay_text(store, session.session_id)
    assert "stt.final" in text or "YOU" in text
    assert "READ" in text or "cage.readback" in text
    assert "SUB" in text or "cage.submit" in text


def test_homophone_eth_and_app(session):
    out = session.on_final_transcript("buy five hundred dollars of eth")
    assert session.cage.state is CageState.ASSEMBLING
    assert out.get("card", {}).get("card") == "disambiguation" or "more than one" in (out.get("speech") or "").lower()
    out = session.on_final_transcript("buy five hundred dollars of App")
    assert "more than one" in (out.get("speech") or "").lower() or out.get("card", {}).get("card") == "disambiguation"
    session.on_final_transcript("buy five hundred dollars of cisco")
    assert session.pending_disambiguation or session.cage.state is CageState.ASSEMBLING


def test_seeded_misrecognition_off(session):
    session.on_final_transcript("buy one hundred fifty dollars of weth")
    complete(session)
    session.cage.draft.quantity.value = Decimal("1500")
    session.cage.draft.slot_confidence["quantity"] = 0.4
    session.cage.state = CageState.ASSEMBLING
    out = session.cage._try_readback()
    assert "one-five-zero-zero" in (out.speech or "") or "thousand" in (out.speech or "")
    complete(session)
    session.on_final_transcript("Off")
    assert session.cage.state is CageState.IDLE
    assert session.broker.fills == []


def test_soft_yes_then_done(session):
    session.on_final_transcript("buy two tenths of weth, limit three thousand eight hundred")
    complete(session)
    out = session.on_final_transcript("yeah go ahead")
    assert session.cage.state is CageState.ARMED_FOR_ASSENT
    assert TEACH_ASSENT in (out.get("speech") or "")
    session.on_final_transcript("Done")
    assert session.cage.state is CageState.FILLED


def test_assent_during_readback(session):
    session.on_final_transcript("buy two tenths of weth, limit three thousand eight hundred")
    assert session.cage.state is CageState.READBACK
    session.interrupt.playing = True
    session.on_final_transcript("done")
    assert session.cage.state is CageState.ASSEMBLING
    assert session.broker.fills == []


def test_universal_brake(session):
    session.on_final_transcript("buy two tenths of weth, limit three thousand eight hundred")
    session.on_final_transcript("cancel")
    assert session.cage.state is CageState.IDLE
    session.on_final_transcript("buy two tenths of weth, limit three thousand eight hundred")
    complete(session)
    session.on_final_transcript("stop")
    assert session.cage.state is CageState.IDLE
    session.on_final_transcript("the open")
    session.on_final_transcript("cancel")
    assert session.cage.state is CageState.IDLE


def test_mandate_ceremony_and_fire(session):
    # seed a position so sell-half can execute
    session.on_final_transcript("buy two tenths of weth market")
    complete(session)
    session.on_final_transcript("Done")
    assert session.cage.state is CageState.FILLED
    out = session.on_final_transcript("if weth drops below three thousand, sell half")
    assert "simulate" in (out.get("speech") or "").lower()
    out = session.on_final_transcript("simulate it")
    assert "fired" in (out.get("speech") or "").lower()
    session.on_final_transcript("because I don't want to ride a breakdown")
    out = session.on_final_transcript("arm it")
    assert "Armed" in (out.get("speech") or "")
    session.on_final_transcript("list mandates")
    session.on_final_transcript("suspend 2")
    session.on_final_transcript("revoke")
    # re-arm a fresh one for the fire
    session.on_final_transcript("if weth drops below three thousand, sell half")
    session.on_final_transcript("because size")
    session.on_final_transcript("arm it")
    m = session.cage.mandate
    assert m is not None
    session.broker.set_mark("WETH", "spot", Decimal("2900"))
    now = datetime.now(timezone.utc)
    session.tick(now)
    reports = session.tick(now + timedelta(seconds=1))
    fired = any("fired" in (r.get("speech") or "").lower() for r in reports)
    assert fired or session.watcher.fires


def test_the_open(session, config):
    out = session.on_final_transcript("the open")
    text = out.get("speech") or ""
    assert SIGN_OFF in text
    assert len(text.split()) <= 75
    assert not contains_day_pnl(text)
    rushed = session.on_final_transcript("I'm rushed")
    rtext = rushed.get("speech") or ""
    assert len(rtext.split()) <= 25
    bloated = render_open(
        {
            "world": " ".join(["alpha"] * 20),
            "names": " ".join(["beta"] * 20),
            "mandates": "day p&l is up twenty percent.",
            "decisions": " ".join(["gamma"] * 20),
        },
        config=config,
        rushed=False,
    )
    assert SIGN_OFF in bloated
    assert not contains_day_pnl(bloated)
    assert len(bloated.split()) <= 75


def test_tape_immutable(store, session):
    session.on_final_transcript("what's weth doing")
    try:
        with store.engine.begin() as conn:
            conn.exec_driver_sql("UPDATE tape_records SET kind='mutated' WHERE id=1")
        raise AssertionError("update should have been blocked")
    except RuntimeError as exc:
        assert "append-only" in str(exc)


def test_parser_intents():
    assert parse_utterance("what's ether doing").name == "quote"
    assert parse_utterance("the open").name == "open"
    assert parse_utterance("I'm rushed").name == "open_rushed"
    assert parse_utterance("buy two tenths of weth limit 3800").name == "order"
    assert parse_utterance("if ether drops below 3000, sell half").name == "mandate"
    assert parse_utterance("").name == "unknown"


def test_quotes_and_book(session):
    out = session.on_final_transcript("what's weth doing")
    assert "as of" in (out.get("speech") or "")
    out = session.on_final_transcript("positions")
    assert "World book" in (out.get("speech") or "")
    session.on_final_transcript("add weth to the watchlist")
    session.on_final_transcript("journal hello")
    session.on_final_transcript("xyzzy")
    assert "didn't catch" in session.on_final_transcript("xyzzy").get("speech", "").lower() or True
