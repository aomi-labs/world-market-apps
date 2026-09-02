from __future__ import annotations

from datetime import datetime, timedelta, timezone
from decimal import Decimal
from typing import Any, Callable, Protocol

from desk.cage.readback import render_readback
from desk.cage.reserved import detect_reserved, is_soft_affirmative, reserved_is_active
from desk.cage.types import (
    CageState,
    CardPayload,
    MandateDraft,
    MandateStatus,
    OrderDraft,
    PortfolioSnapshot,
    SpokenText,
    ValidationIssue,
)
from desk.cage.validate import PolicyMandate, _notional, resolve_base_quantity, validate_mandate, validate_order
from desk.config import DeskConfig


Clock = Callable[[], datetime]


def _utcnow() -> datetime:
    return datetime.now(timezone.utc)


class Tape(Protocol):
    def record(self, kind: str, payload: dict[str, Any]) -> None: ...


class Broker(Protocol):
    def snapshot(self) -> PortfolioSnapshot: ...

    def mark(self, symbol: str, product: str) -> Decimal | None: ...

    def spread_bps(self, symbol: str, product: str) -> Decimal | None: ...

    def submit(self, draft: OrderDraft, base_qty: Decimal) -> dict[str, Any]: ...

    def cancel_in_flight(self, order_id: str) -> bool: ...


class CageResult:
    def __init__(
        self,
        *,
        state: CageState,
        speech: str | None = None,
        card: CardPayload | None = None,
        submitted: dict[str, Any] | None = None,
        consumed: bool = True,
        teach_assent: bool = False,
        issues: list[ValidationIssue] | None = None,
        readback: SpokenText | None = None,
        flush_tts: bool = False,
        earcon: str | None = None,
    ) -> None:
        self.state = state
        self.speech = speech
        self.card = card
        self.submitted = submitted
        self.consumed = consumed
        self.teach_assent = teach_assent
        self.issues = issues or []
        self.readback = readback
        self.flush_tts = flush_tts
        self.earcon = earcon


TEACH_ASSENT = "Say 'Done' to place it — that's the only word that moves money."
PARK_LINE = "I'll hold the ticket — it's on your screen."
BRAKE_LINE = "Stopped. Ticket cleared."
SOFT_FILL = "Filled."


class Cage:
    """Deterministic money layer. The LLM cannot bypass this object."""

    def __init__(
        self,
        config: DeskConfig,
        broker: Broker,
        tape: Tape,
        *,
        policy: PolicyMandate | None = None,
        clock: Clock = _utcnow,
    ) -> None:
        self.config = config
        self.broker = broker
        self.tape = tape
        self.policy = policy
        self.clock = clock
        self.state = CageState.IDLE
        self.draft: OrderDraft | None = None
        self.mandate: MandateDraft | None = None
        self.instrument_chosen = False
        self.readback_text: SpokenText | None = None
        self.readback_complete = False
        self.assent_deadline: datetime | None = None
        self.taught_assent = False
        self.broker_order_id: str | None = None
        self.last_speech: str | None = None
        self.tts_rate: float = 1.0

    def _emit(self, kind: str, extra: dict[str, Any] | None = None) -> None:
        payload = {
            "state": self.state.value,
            "draft": self.draft.model_dump(mode="json") if self.draft else None,
            "mandate": self.mandate.model_dump(mode="json") if self.mandate else None,
            **(extra or {}),
        }
        self.tape.record(kind, payload)

    def _transition(self, new: CageState, reason: str) -> None:
        old = self.state
        self.state = new
        self._emit(
            "cage.transition",
            {"from": old.value, "to": new.value, "reason": reason},
        )

    def ticket_card(self, state: str | None = None) -> CardPayload | None:
        if self.draft is None:
            return None
        d = self.draft
        inst = d.instrument
        slots = {
            "side": d.side,
            "quantity": None
            if d.quantity is None
            else {"kind": d.quantity.kind.value, "value": str(d.quantity.value)},
            "instrument": None
            if inst is None
            else {"symbol": inst.symbol, "name": inst.name, "product": inst.product},
            "order_type": d.order_type,
            "limit_price": str(d.limit_price) if d.limit_price is not None else None,
            "stop_price": str(d.stop_price) if d.stop_price is not None else None,
            "duration": d.duration,
        }
        card_state = state or {
            CageState.ASSEMBLING: "assembling",
            CageState.READBACK: "readback",
            CageState.ARMED_FOR_ASSENT: "readback",
            CageState.PARKED: "assembling",
            CageState.SUBMITTED: "stamped",
            CageState.WORKING: "stamped",
            CageState.FILLED: "stamped",
            CageState.CANCELLED: "stamped",
        }.get(self.state, "assembling")
        book = self.broker.snapshot()
        mark = self.broker.mark(inst.symbol, inst.product) if inst else None
        notional = _notional(d, mark)
        consequence = None
        if notional and book.equity > 0:
            pct = (notional / book.equity) * Decimal(100)
            consequence = f"{pct.quantize(Decimal('0.1'))}% of the book"
        return CardPayload(
            card="ticket",
            state=card_state,
            slots=slots,
            confidence=dict(d.slot_confidence),
            consequence=consequence,
            ticket_id=d.id,
        )

    def propose_order(self, partial: OrderDraft) -> CageResult:
        if self.state in {CageState.SUBMITTED, CageState.WORKING}:
            return CageResult(
                state=self.state,
                speech="There's already a working order. Cancel it first.",
                consumed=True,
            )
        if self.state in {CageState.IDLE, CageState.CANCELLED, CageState.FILLED, CageState.PARKED}:
            self.draft = partial
            self.instrument_chosen = False
            self.taught_assent = False
            self.readback_complete = False
            self._transition(CageState.ASSEMBLING, "propose_order")
        else:
            assert self.draft is not None
            self.draft = self.draft.merge(partial)
            if self.state in {CageState.READBACK, CageState.ARMED_FOR_ASSENT}:
                self.readback_complete = False
                self._transition(CageState.ASSEMBLING, "amendment")
        self._emit("cage.propose_order")
        return self._try_readback()

    def choose_instrument(self, instrument: Any) -> CageResult:
        if self.draft is None:
            return CageResult(state=self.state, consumed=True, speech="There's no ticket yet.")
        self.draft.instrument = instrument
        self.draft.slot_confidence["instrument"] = max(
            instrument.confidence, self.config.instrument_confidence_threshold
        )
        self.instrument_chosen = True
        self._transition(CageState.ASSEMBLING, "instrument_chosen")
        return self._try_readback()

    def _try_readback(self) -> CageResult:
        assert self.draft is not None
        book = self.broker.snapshot()
        inst = self.draft.instrument
        mark = self.broker.mark(inst.symbol, inst.product) if inst else None
        spread = self.broker.spread_bps(inst.symbol, inst.product) if inst else None
        issues = validate_order(
            self.draft,
            config=self.config,
            book=book,
            mark=mark,
            spread_bps=spread,
            instrument_chosen=self.instrument_chosen,
            policy=self.policy,
        )
        card = self.ticket_card("assembling")
        if issues:
            spoken = issues[0].spoken
            self.last_speech = spoken
            return CageResult(
                state=self.state,
                speech=spoken,
                card=card,
                issues=issues,
            )
        notional = _notional(self.draft, mark)
        spoken = render_readback(
            self.draft,
            config=self.config,
            equity=book.equity,
            notional=notional,
            spread_bps=spread,
        )
        self.readback_text = spoken
        self.readback_complete = False
        self._transition(CageState.READBACK, "validated")
        self.last_speech = spoken.text
        self._emit("cage.readback", {"text": spoken.text})
        return CageResult(
            state=self.state,
            speech=spoken.text,
            card=self.ticket_card("readback"),
            readback=spoken,
        )

    def notify_tts_playout(self, *, completed: bool, heard_up_to_word: int | None = None) -> CageResult:
        self._emit(
            "cage.tts_playout",
            {"completed": completed, "heard_up_to_word": heard_up_to_word},
        )
        if self.state != CageState.READBACK:
            return CageResult(state=self.state, consumed=True)
        if completed:
            self.readback_complete = True
            self.assent_deadline = self.clock() + timedelta(seconds=self.config.assent_timeout_sec)
            self._transition(CageState.ARMED_FOR_ASSENT, "readback_complete")
            return CageResult(state=self.state, card=self.ticket_card("readback"))
        return CageResult(state=self.state)

    def barge_in(self, heard_up_to_word: int | None = None) -> CageResult:
        self._emit("cage.barge_in", {"heard_up_to_word": heard_up_to_word})
        if self.state == CageState.READBACK:
            self.readback_complete = False
            self._transition(CageState.ASSEMBLING, "barge_in_during_readback")
            return CageResult(
                state=self.state,
                card=self.ticket_card("assembling"),
                flush_tts=True,
                speech=None,
            )
        return CageResult(state=self.state, flush_tts=True)

    def tick(self, now: datetime | None = None) -> CageResult | None:
        now = now or self.clock()
        if self.state == CageState.ARMED_FOR_ASSENT and self.assent_deadline and now >= self.assent_deadline:
            return self._park("timeout")
        return None

    def handle_transcript(self, text: str) -> CageResult:
        word = detect_reserved(text)
        if word and reserved_is_active(word, self.state):
            return self._handle_reserved(word)
        if self.state == CageState.ARMED_FOR_ASSENT and is_soft_affirmative(text):
            self._emit("cage.soft_affirmative", {"text": text})
            speech = TEACH_ASSENT if not self.taught_assent else TEACH_ASSENT
            self.taught_assent = True
            self.last_speech = speech
            return CageResult(state=self.state, speech=speech, teach_assent=True)
        if self.state in {CageState.READBACK, CageState.ARMED_FOR_ASSENT}:
            self.readback_complete = False
            self._transition(CageState.ASSEMBLING, "amendment_utterance")
            return CageResult(state=self.state, consumed=False, card=self.ticket_card("assembling"))
        return CageResult(state=self.state, consumed=False)

    def _handle_reserved(self, word: str) -> CageResult:
        self._emit("cage.reserved", {"word": word})
        if word in {"cancel", "stop"}:
            return self._brake()
        if word == "done":
            return self._done()
        if word == "off":
            return self._off()
        if word == "hold":
            return self._park("hold")
        if word == "repeat":
            speech = self.last_speech or (self.readback_text.text if self.readback_text else "Nothing to repeat.")
            return CageResult(state=self.state, speech=speech)
        if word == "slower":
            self.tts_rate = max(0.7, self.tts_rate - 0.1)
            return CageResult(state=self.state, speech="Slower.")
        if word == "show me":
            return CageResult(state=self.state, card=self.ticket_card(), speech=None)
        return CageResult(state=self.state, consumed=False)

    def _done(self) -> CageResult:
        if self.state == CageState.READBACK and not self.readback_complete:
            return self.barge_in()
        if self.state != CageState.ARMED_FOR_ASSENT:
            return CageResult(state=self.state, consumed=False)
        assert self.draft is not None
        book = self.broker.snapshot()
        inst = self.draft.instrument
        assert inst is not None
        mark = self.broker.mark(inst.symbol, inst.product)
        issues = validate_order(
            self.draft,
            config=self.config,
            book=book,
            mark=mark,
            instrument_chosen=self.instrument_chosen or inst.confidence >= self.config.instrument_confidence_threshold,
            policy=self.policy,
        )
        if issues:
            self._transition(CageState.ASSEMBLING, "revalidate_failed")
            return CageResult(state=self.state, speech=issues[0].spoken, issues=issues, card=self.ticket_card())
        pos_qty = Decimal(0)
        for pos in book.positions:
            if pos.symbol == inst.symbol and pos.product == inst.product:
                pos_qty = pos.quantity
                break
        base_qty, qty_issue = resolve_base_quantity(self.draft, mark=mark, position_qty=pos_qty)
        if qty_issue or base_qty is None:
            return CageResult(state=self.state, speech=(qty_issue.spoken if qty_issue else "Can't size it."))
        receipt = self.broker.submit(self.draft, base_qty)
        self.broker_order_id = str(receipt.get("order_id", ""))
        self._transition(CageState.SUBMITTED, "done")
        self._emit("cage.submit", {"receipt": receipt})
        status = str(receipt.get("status", "working"))
        if status == "filled":
            self._transition(CageState.FILLED, "fill")
            fill_px = receipt.get("fill_price")
            speech = self._fill_speech(fill_px)
            self.last_speech = speech
            return CageResult(
                state=self.state,
                speech=speech,
                card=self.ticket_card("stamped"),
                submitted=receipt,
                earcon="fill",
            )
        self._transition(CageState.WORKING, "acked")
        speech = "Working."
        self.last_speech = speech
        return CageResult(
            state=self.state,
            speech=speech,
            card=self.ticket_card("stamped"),
            submitted=receipt,
        )

    def _fill_speech(self, fill_px: Any) -> str:
        d = self.draft
        assert d is not None and d.instrument is not None and d.side is not None
        from desk.speech import speak_price

        px = speak_price(Decimal(str(fill_px)), verbosity=self.config.verbosity) if fill_px is not None else "the mark"
        verb = "Bought" if d.side == "buy" else "Sold"
        return f"{verb} {d.instrument.name} at {px}."

    def _off(self) -> CageResult:
        if self.state not in {CageState.ARMED_FOR_ASSENT, CageState.READBACK, CageState.ASSEMBLING}:
            return CageResult(state=self.state, consumed=False)
        self._transition(CageState.CANCELLED, "off")
        speech = "Off. Ticket cancelled."
        self.last_speech = speech
        card = self.ticket_card("stamped")
        self.draft = None
        self.readback_text = None
        self._transition(CageState.IDLE, "cleared")
        return CageResult(state=self.state, speech=speech, card=card, flush_tts=True)

    def _park(self, reason: str) -> CageResult:
        self._transition(CageState.PARKED, reason)
        self.last_speech = PARK_LINE
        self._emit("cage.parked", {"reason": reason})
        return CageResult(
            state=self.state,
            speech=PARK_LINE,
            card=self.ticket_card("assembling"),
            flush_tts=True,
        )

    def resume_parked(self) -> CageResult:
        if self.state != CageState.PARKED or self.draft is None:
            return CageResult(state=self.state, speech="Nothing parked.")
        self._transition(CageState.ASSEMBLING, "resume")
        return self._try_readback()

    def _brake(self) -> CageResult:
        cancelled = False
        if self.broker_order_id and self.state in {CageState.SUBMITTED, CageState.WORKING}:
            cancelled = self.broker.cancel_in_flight(self.broker_order_id)
        self._emit("cage.brake", {"cancelled_in_flight": cancelled})
        self._transition(CageState.CANCELLED, "brake")
        self.draft = None
        self.mandate = None
        self.readback_text = None
        self.readback_complete = False
        self.broker_order_id = None
        self._transition(CageState.IDLE, "brake_idle")
        self.last_speech = BRAKE_LINE
        return CageResult(
            state=self.state,
            speech=BRAKE_LINE,
            card=CardPayload(card="ticket", state="stamped", slots={}),
            flush_tts=True,
        )

    def propose_mandate(self, partial: MandateDraft) -> CageResult:
        if self.mandate is None or self.mandate.status is not MandateStatus.DRAFT:
            self.mandate = partial
        else:
            self.mandate = self.mandate.merge(partial)
        self._apply_mandate_defaults()
        self._emit("cage.propose_mandate")
        issues = validate_mandate(self.mandate, config=self.config)
        missing = [i for i in issues if i.code == "missing_slot" or i.code == "low_confidence"]
        if missing:
            return CageResult(state=self.state, speech=missing[0].spoken, issues=issues)
        return CageResult(
            state=self.state,
            speech=self.paraphrase_mandate(),
            card=self.registry_card(),
        )

    def _apply_mandate_defaults(self) -> None:
        assert self.mandate is not None
        action = self.mandate.action
        trig = self.mandate.trigger
        if action.order_type is None or action.order_type == "market":
            action.order_type = "limit"
        if action.limit_price is None and trig.price is not None:
            bps = self.config.limit_offset_bps / Decimal(10_000)
            if trig.comparator in {"lt", "lte"}:
                action.limit_price = trig.price * (Decimal(1) - bps)
            else:
                action.limit_price = trig.price * (Decimal(1) + bps)
        if action.duration != "gtc":
            action.duration = "day"
        if action.instrument is None and trig.instrument is not None:
            action.instrument = trig.instrument
        if action.instrument_query is None:
            action.instrument_query = trig.instrument_query
        if not self.mandate.name and trig.instrument is not None:
            self.mandate.name = f"{trig.instrument.symbol} {trig.comparator} {trig.price}"
        if self.mandate.confirmation_window_sec is None:
            self.mandate.confirmation_window_sec = self.config.mandate_confirmation_window_sec
        if self.mandate.expiry_days is None:
            self.mandate.expiry_days = self.config.mandate_expiry_days

    def paraphrase_mandate(self) -> str:
        m = self.mandate
        assert m is not None
        trig = m.trigger
        inst = trig.instrument
        name = inst.name if inst else trig.instrument_query
        cmp_word = {
            "lt": "drops below",
            "lte": "drops to or below",
            "gt": "rises above",
            "gte": "rises to or above",
        }[trig.comparator]
        from desk.speech import speak_price

        px = speak_price(trig.price, verbosity=self.config.verbosity) if trig.price is not None else "the trigger"
        action = m.action
        side = action.side or "sell"
        qty = action.quantity
        if qty and qty.kind.value == "pct_of_position":
            size = "half" if qty.value in {Decimal("0.5"), Decimal("50")} else f"{qty.value} percent"
        elif qty:
            size = str(qty.value)
        else:
            size = "the size"
        limit = (
            speak_price(action.limit_price, verbosity=self.config.verbosity)
            if action.limit_price is not None
            else "the trigger minus ten basis points"
        )
        return (
            f"If {name} {cmp_word} {px}, I'll {side} {size}, limit {limit}, "
            f"day order at trigger, expires in {m.expiry_days} days. "
            f"{m.confirmation_window_sec} second confirmation window. That's the rule — shall I simulate it?"
        )

    def set_rationale(self, text: str) -> None:
        if self.mandate is None:
            return
        self.mandate.rationale_text = text.strip()
        self._emit("cage.rationale", {"text": self.mandate.rationale_text})

    def arm_mandate(self) -> CageResult:
        if self.mandate is None:
            return CageResult(state=self.state, speech="There's no rule to arm.")
        self.mandate.status = MandateStatus.ARMED
        issues = validate_mandate(self.mandate, config=self.config)
        if issues:
            self.mandate.status = MandateStatus.DRAFT
            return CageResult(state=self.state, speech=issues[0].spoken, issues=issues)
        now = self.clock()
        self.mandate.expires_at = now + timedelta(days=self.mandate.expiry_days)
        self.mandate.status = MandateStatus.ARMED
        self._emit("cage.mandate_armed")
        speech = f"Armed. It's on the registry. {self.mandate.name}."
        self.last_speech = speech
        return CageResult(state=self.state, speech=speech, card=self.registry_card())

    def registry_card(self) -> CardPayload:
        m = self.mandate
        return CardPayload(
            card="registry",
            state=m.status.value if m else "empty",
            payload={"mandate": m.model_dump(mode="json") if m else None},
        )

    def notify_fill(self, receipt: dict[str, Any]) -> CageResult:
        if self.state not in {CageState.SUBMITTED, CageState.WORKING}:
            return CageResult(state=self.state)
        self._transition(CageState.FILLED, "broker_fill")
        speech = self._fill_speech(receipt.get("fill_price"))
        self.last_speech = speech
        self._emit("cage.fill", {"receipt": receipt})
        return CageResult(
            state=self.state,
            speech=speech,
            card=self.ticket_card("stamped"),
            earcon="fill",
        )
