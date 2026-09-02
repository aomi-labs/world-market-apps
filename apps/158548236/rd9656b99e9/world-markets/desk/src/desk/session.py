from __future__ import annotations

import math
import time
from datetime import datetime, timezone
from decimal import Decimal
from typing import Any, Callable
from uuid import uuid4

from desk.cage import Cage, CageResult, MandateDraft, OrderDraft, ResolvedInstrument, TEACH_ASSENT
from desk.config import DeskConfig
from desk.instruments import InstrumentResolver
from desk.interrupt import InterruptionTracker
from desk.mandates import MandateWatcher
from desk.open_liturgy import default_bundle, render_open
from desk.parser import parse_utterance
from desk.persist import TapeLogger
from desk.policy import AomiPolicy
from desk.speech import speak_price, speak_text
from desk.trading import make_broker

Push = Callable[[dict[str, Any]], None]


class DeskSession:
    def __init__(
        self,
        config: DeskConfig,
        *,
        tape: TapeLogger,
        broker: Any | None = None,
        resolver: InstrumentResolver | None = None,
        policy: AomiPolicy | None = None,
        push: Push | None = None,
        clock: Callable[[], datetime] | None = None,
    ) -> None:
        self.config = config
        self.tape = tape
        self.broker = broker or make_broker(config)
        universe = getattr(self.broker, "universe", lambda: None)()
        self.resolver = resolver or (
            InstrumentResolver(rows=universe) if universe else InstrumentResolver()
        )
        self.policy = policy or AomiPolicy.from_path(config.aomi_mandate_path)
        self.push = push or (lambda _msg: None)
        self.clock = clock or (lambda: datetime.now(timezone.utc))
        self.cage = Cage(config, self.broker, tape, policy=self.policy, clock=self.clock)
        self.watcher = MandateWatcher(config, self.broker, self.cage, clock=self.clock)
        self.interrupt = InterruptionTracker()
        self.watchlist = list(config.watchlist)
        self.last_instrument: ResolvedInstrument | None = None
        self.pending_disambiguation: list[ResolvedInstrument] = []
        self.journal: list[str] = []
        self.latency: list[dict[str, float]] = []
        self.session_id = tape.session_id

    def on_final_transcript(self, text: str, *, words: list[dict[str, Any]] | None = None) -> dict[str, Any]:
        t0 = time.perf_counter()
        self.tape.record("stt.final", {"text": text, "words": words or []})
        if self.interrupt.playing:
            note = self.interrupt.context_note()
            self.tape.record("interrupt", {"note": note, **self.interrupt.barge_in()})
            self._flush_tts()
        result = self.cage.handle_transcript(text)
        if result.consumed:
            out = self._apply_result(result)
            self._span("turn", t0)
            return out
        intent = parse_utterance(text)
        self.tape.record("nlu.intent", {"name": intent.name, "fields": _safe(intent.fields)})
        out = self._dispatch(intent, text)
        self._span("turn", t0)
        return out

    def notify_tts_complete(self) -> dict[str, Any]:
        self.interrupt.complete()
        result = self.cage.notify_tts_playout(completed=True)
        return self._apply_result(result)

    def notify_tts_marker(self, char_offset: int) -> None:
        self.interrupt.on_marker(char_offset)

    def tick(self, now: datetime | None = None) -> list[dict[str, Any]]:
        outs: list[dict[str, Any]] = []
        parked = self.cage.tick(now)
        if parked:
            outs.append(self._apply_result(parked))
        for report in self.watcher.poll(now):
            outs.append(self._speak(report, earcon="chime"))
        return outs

    def _dispatch(self, intent: Any, text: str) -> dict[str, Any]:
        name = intent.name
        if name == "unknown":
            if self.pending_disambiguation:
                choice = self._match_choice(text)
                if choice:
                    self.pending_disambiguation = []
                    return self._apply_result(self.cage.choose_instrument(choice))
            return self._speak("I didn't catch that.")
        if name == "quote":
            return self._quote(intent.fields.get("query") or "")
        if name == "positions":
            return self._positions()
        if name == "order":
            return self._order(intent.fields["draft"], intent.fields.get("product"), intent.fields.get("query"))
        if name == "mandate":
            return self._mandate(intent.fields["mandate"], intent.fields.get("query"))
        if name == "simulate":
            return self._simulate()
        if name == "arm":
            return self._arm()
        if name == "rationale":
            self.cage.set_rationale(intent.fields.get("text") or text)
            return self._speak("Recorded. Say 'arm' to put it on the registry.")
        if name == "list_mandates":
            return self._list_mandates()
        if name == "suspend":
            return self._suspend(int(intent.fields.get("hours") or 24))
        if name == "revoke":
            return self._revoke()
        if name == "open":
            return self._open(rushed=False)
        if name == "open_rushed":
            return self._open(rushed=True)
        if name == "watchlist_add":
            return self._watch(intent.fields.get("query") or "", add=True)
        if name == "watchlist_remove":
            return self._watch(intent.fields.get("query") or "", add=False)
        if name == "journal":
            self.journal.append(intent.fields.get("text") or text)
            self.tape.record("journal", {"text": intent.fields.get("text")})
            return self._speak("Noted.")
        if name == "resume_ticket":
            return self._apply_result(self.cage.resume_parked())
        return self._speak("I didn't catch that.")

    def _order(self, draft: OrderDraft, product: str | None, query: str | None) -> dict[str, Any]:
        q = query or draft.instrument_query or ""
        if q in {"it", "that"} and self.last_instrument:
            draft.instrument = self.last_instrument
            draft.slot_confidence["instrument"] = self.last_instrument.confidence
        else:
            cands = self.resolver.search(q, product=product or self.config.default_product if q else None)
            # If product wasn't specified, search both.
            if not cands and not product:
                cands = self.resolver.search(q)
            if not cands:
                draft.instrument_query = q
                self.cage.propose_order(draft)
                return self._speak(f"I don't have {q or 'that name'} on the World book.")
            if len(cands) > 1 or cands[0].instrument.confidence < self.config.instrument_confidence_threshold:
                self.pending_disambiguation = [c.instrument for c in cands]
                draft.instrument = cands[0].instrument
                draft.instrument_query = q
                self.cage.propose_order(draft)
                return self._disambiguate(cands)
            draft.instrument = cands[0].instrument
            draft.slot_confidence["instrument"] = cands[0].instrument.confidence
            self.last_instrument = draft.instrument
        return self._apply_result(self.cage.propose_order(draft))

    def _mandate(self, mandate: MandateDraft, query: str | None) -> dict[str, Any]:
        q = query or mandate.trigger.instrument_query
        if q in {"it", "that"} and self.last_instrument:
            inst = self.last_instrument
        else:
            cands = self.resolver.search(q or "", product=self.config.default_product)
            if not cands:
                cands = self.resolver.search(q or "")
            if not cands:
                return self._speak("I don't have that name.")
            if len(cands) > 1 and cands[0].instrument.confidence < self.config.instrument_confidence_threshold:
                self.pending_disambiguation = [c.instrument for c in cands]
                return self._disambiguate(cands)
            inst = cands[0].instrument
        mandate.trigger.instrument = inst
        mandate.action.instrument = inst
        self.last_instrument = inst
        result = self.cage.propose_mandate(mandate)
        return self._apply_result(result)

    def _simulate(self) -> dict[str, Any]:
        m = self.cage.mandate
        if m is None or m.trigger.instrument is None or m.trigger.price is None:
            return self._speak("There's no rule to simulate.")
        fires = self.broker.simulate_trigger(
            m.trigger.instrument.symbol, m.trigger.comparator, m.trigger.price
        )
        n = len(fires)
        dates = ", ".join(d.strftime("%b %d") for d in fires[:3])
        extra = f" First dates: {dates}." if dates else ""
        speech = f"Would have fired {n} times in the last month.{extra} In your words — why does this rule exist?"
        self.tape.record("mandate.simulate", {"n": n, "dates": [d.isoformat() for d in fires]})
        return self._speak(speech)

    def _arm(self) -> dict[str, Any]:
        result = self.cage.arm_mandate()
        if result.issues:
            return self._apply_result(result)
        if self.cage.mandate:
            self.watcher.register(self.cage.mandate)
            register = getattr(self.broker, "register_watch", None)
            if callable(register):
                register(self.cage.mandate)
        return self._apply_result(result)

    def _list_mandates(self) -> dict[str, Any]:
        items = [m for m in self.watcher.armed.values()]
        if not items:
            return self._speak("Registry is empty.")
        names = "; ".join(f"{m.name} ({m.status.value})" for m in items)
        return self._speak(names, card=self.cage.registry_card().model_dump())

    def _suspend(self, hours: int) -> dict[str, Any]:
        if not self.cage.mandate:
            return self._speak("No rule on the table.")
        self.watcher.suspend(self.cage.mandate.id, hours)
        return self._speak(f"Suspended for {hours} hours.")

    def _revoke(self) -> dict[str, Any]:
        if not self.cage.mandate:
            return self._speak("No rule on the table.")
        self.watcher.revoke(self.cage.mandate.id)
        drop = getattr(self.broker, "drop_watch", None)
        if callable(drop):
            drop(self.cage.mandate.id)
        return self._speak("Revoked.")

    def _quote(self, query: str) -> dict[str, Any]:
        cands = self.resolver.search(query, product=self.config.default_product) or self.resolver.search(query)
        if not cands:
            return self._speak("I don't have that name.")
        inst = cands[0].instrument
        self.last_instrument = inst
        q = self.broker.quote(inst.symbol, inst.product, inst.name)
        if q is None:
            return self._speak("No mark.")
        age = max(1, int(q.age_seconds()) or 1)
        spoken = (
            f"{inst.name} is {speak_price(q.mark, verbosity=self.config.verbosity)} "
            f"as of {age} seconds ago."
        )
        card = {
            "card": "book",
            "state": "quote",
            "payload": q.model_dump(mode="json"),
        }
        return self._speak(spoken, card=card)

    def _positions(self) -> dict[str, Any]:
        snap = self.broker.snapshot()
        if not snap.positions:
            speech = f"World book. Cash {speak_price(snap.cash, verbosity=self.config.verbosity)}. Flat."
        else:
            bits = [f"{p.quantity} {p.symbol} {p.product}" for p in snap.positions[:3]]
            speech = "World book: " + "; ".join(bits) + "."
            if len(snap.positions) > 3:
                speech = speech[:-1] + " — rest is on the card."
        return self._speak(
            speech,
            card={"card": "book", "state": "positions", "payload": snap.model_dump(mode="json")},
        )

    def _open(self, rushed: bool) -> dict[str, Any]:
        quotes = []
        for sym in self.watchlist:
            q = self.broker.quote(sym, "spot", sym)
            if q:
                quotes.append(q)
        overnight = None
        if self.watcher.fires:
            overnight = f"{len(self.watcher.fires)} mandate fire(s) since last session."
        elif self.watcher.queued_reports:
            overnight = self.watcher.queued_reports[0]
        notes = getattr(self.broker, "open_notes", lambda: {})()
        if isinstance(notes, dict):
            if not overnight and notes.get("ledger"):
                overnight = notes["ledger"]
            decision = notes.get("pnl")
        else:
            decision = None
        bundle = default_bundle(
            quotes=quotes,
            watchlist=self.watchlist,
            mandate_notes=overnight,
            decision=decision,
        )
        text = render_open(bundle, config=self.config, rushed=rushed)
        self.tape.record("open", {"text": text, "rushed": rushed, "bundle": bundle})
        return self._speak(text, card={"card": "queue", "state": "open", "payload": bundle}, earcon=None)

    def _watch(self, query: str, *, add: bool) -> dict[str, Any]:
        cands = self.resolver.search(query)
        if not cands:
            return self._speak("I don't have that name.")
        sym = cands[0].instrument.symbol
        if add:
            if sym not in self.watchlist:
                self.watchlist.append(sym)
            return self._speak(f"{sym} is on the watchlist.")
        self.watchlist = [s for s in self.watchlist if s != sym]
        return self._speak(f"{sym} is off the watchlist.")

    def _disambiguate(self, cands: Any) -> dict[str, Any]:
        bits = []
        for c in cands[:4]:
            inst = c.instrument
            px = inst.last_price
            px_s = speak_price(px, verbosity="expert") if px is not None else "no mark"
            bits.append(f"{inst.name}, {inst.product}, around {px_s}")
        speech = "I have more than one. " + "; ".join(bits) + ". Which one?"
        card = {
            "card": "disambiguation",
            "state": "choose",
            "payload": {"candidates": [c.instrument.model_dump(mode="json") for c in cands]},
        }
        return self._speak(speech, card=card)

    def _match_choice(self, text: str) -> ResolvedInstrument | None:
        t = text.lower()
        for inst in self.pending_disambiguation:
            if inst.product in t or inst.symbol.lower() in t or inst.name.lower() in t:
                inst.confidence = 0.99
                return inst
        return None

    def _apply_result(self, result: CageResult) -> dict[str, Any]:
        out: dict[str, Any] = {"state": result.state.value, "consumed": result.consumed}
        if result.flush_tts:
            self._flush_tts()
            out["flush_tts"] = True
        if result.card:
            payload = result.card.model_dump()
            self.tape.record("card.push", payload)
            self.push({"type": "card", **payload})
            out["card"] = payload
        if result.speech:
            out.update(self._speak(result.speech, earcon=result.earcon, skip_push_card=True))
        elif result.earcon:
            self.push({"type": "earcon", "name": result.earcon})
            out["earcon"] = result.earcon
        if result.submitted:
            out["submitted"] = result.submitted
        if result.teach_assent:
            out["teach_assent"] = True
        return out

    def _speak(
        self,
        text: str,
        *,
        card: dict[str, Any] | None = None,
        earcon: str | None = None,
        extra: bool = False,
        skip_push_card: bool = False,
    ) -> dict[str, Any]:
        spoken = speak_text(text, verbosity=self.config.verbosity, state=self.cage.state.value)
        self.interrupt.start(spoken)
        self.tape.record("tts.speak", {"text": spoken})
        self.push({"type": "speech", "text": spoken, "rate": self.cage.tts_rate})
        out: dict[str, Any] = {"speech": spoken, "state": self.cage.state.value}
        if card and not skip_push_card:
            self.tape.record("card.push", card)
            self.push({"type": "card", **card})
            out["card"] = card
        if earcon:
            self.push({"type": "earcon", "name": earcon})
            out["earcon"] = earcon
        _ = extra
        return out

    def _flush_tts(self) -> None:
        self.interrupt.playing = False
        self.push({"type": "flush_tts"})

    def _span(self, name: str, t0: float) -> None:
        ms = (time.perf_counter() - t0) * 1000
        rec = {name + "_ms": ms}
        self.latency.append(rec)
        self.tape.record("latency", rec)

    def latency_report(self) -> str:
        if not self.latency:
            return "no turns"
        vals = [row.get("turn_ms", 0.0) for row in self.latency]
        vals.sort()
        median = vals[len(vals) // 2]
        p95 = vals[min(len(vals) - 1, math.ceil(0.95 * len(vals)) - 1)]
        return f"turns={len(vals)} median_ms={median:.0f} p95_ms={p95:.0f}"


def _safe(fields: dict[str, Any]) -> dict[str, Any]:
    out = {}
    for k, v in fields.items():
        if hasattr(v, "model_dump"):
            out[k] = v.model_dump(mode="json")
        else:
            out[k] = v
    return out


def new_session(config: DeskConfig, store: Any, session_id: str | None = None) -> DeskSession:
    sid = session_id or uuid4().hex[:10]
    tape = TapeLogger(store, sid)
    tape.record("session.start", {"config": config.model_dump(mode="json")})
    return DeskSession(config, tape=tape)
