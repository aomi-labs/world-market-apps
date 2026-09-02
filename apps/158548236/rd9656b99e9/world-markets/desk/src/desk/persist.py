from __future__ import annotations

from datetime import datetime, timezone
from typing import Any, Callable

from sqlalchemy import JSON, DateTime, Integer, String, Text, create_engine, event, select
from sqlalchemy.orm import DeclarativeBase, Mapped, mapped_column, Session


class Base(DeclarativeBase):
    pass


class TapeRecord(Base):
    __tablename__ = "tape_records"
    id: Mapped[int] = mapped_column(Integer, primary_key=True, autoincrement=True)
    session_id: Mapped[str] = mapped_column(String(64), index=True)
    seq: Mapped[int] = mapped_column(Integer)
    kind: Mapped[str] = mapped_column(String(64))
    ts: Mapped[datetime] = mapped_column(DateTime(timezone=True))
    payload: Mapped[dict[str, Any]] = mapped_column(JSON)


class OrderDraftRow(Base):
    __tablename__ = "order_drafts"
    id: Mapped[str] = mapped_column(String(32), primary_key=True)
    session_id: Mapped[str] = mapped_column(String(64), index=True)
    state: Mapped[str] = mapped_column(String(32))
    body: Mapped[dict[str, Any]] = mapped_column(JSON)


class OrderRow(Base):
    __tablename__ = "orders"
    id: Mapped[str] = mapped_column(String(32), primary_key=True)
    draft_id: Mapped[str] = mapped_column(String(32))
    broker_order_id: Mapped[str | None] = mapped_column(String(64), nullable=True)
    status: Mapped[str] = mapped_column(String(32))
    terms: Mapped[dict[str, Any]] = mapped_column(JSON)
    events: Mapped[list[Any]] = mapped_column(JSON, default=list)


class ExecutionRow(Base):
    __tablename__ = "executions"
    id: Mapped[int] = mapped_column(Integer, primary_key=True, autoincrement=True)
    order_id: Mapped[str] = mapped_column(String(32), index=True)
    body: Mapped[dict[str, Any]] = mapped_column(JSON)


class MandateRow(Base):
    __tablename__ = "mandates"
    id: Mapped[str] = mapped_column(String(32), primary_key=True)
    status: Mapped[str] = mapped_column(String(32))
    body: Mapped[dict[str, Any]] = mapped_column(JSON)
    rationale_text: Mapped[str | None] = mapped_column(Text, nullable=True)
    rationale_audio_ref: Mapped[str | None] = mapped_column(String(256), nullable=True)


class MandateFireRow(Base):
    __tablename__ = "mandate_fires"
    id: Mapped[int] = mapped_column(Integer, primary_key=True, autoincrement=True)
    mandate_id: Mapped[str] = mapped_column(String(32), index=True)
    fire_time: Mapped[datetime] = mapped_column(DateTime(timezone=True))
    resulting_order_id: Mapped[str | None] = mapped_column(String(32), nullable=True)
    report_delivered_at: Mapped[datetime | None] = mapped_column(DateTime(timezone=True), nullable=True)
    body: Mapped[dict[str, Any]] = mapped_column(JSON)


class WatchlistRow(Base):
    __tablename__ = "watchlist"
    symbol: Mapped[str] = mapped_column(String(16), primary_key=True)


class JournalRow(Base):
    __tablename__ = "journal_entries"
    id: Mapped[int] = mapped_column(Integer, primary_key=True, autoincrement=True)
    text: Mapped[str] = mapped_column(Text)
    linked_order_id: Mapped[str | None] = mapped_column(String(32), nullable=True)
    ts: Mapped[datetime] = mapped_column(DateTime(timezone=True))


class UserConfigRow(Base):
    __tablename__ = "user_config"
    key: Mapped[str] = mapped_column(String(64), primary_key=True)
    value: Mapped[str] = mapped_column(Text)


class InstrumentRowSQL(Base):
    __tablename__ = "instruments"
    id: Mapped[int] = mapped_column(Integer, primary_key=True, autoincrement=True)
    symbol: Mapped[str] = mapped_column(String(16), index=True)
    name: Mapped[str] = mapped_column(String(128))
    product: Mapped[str] = mapped_column(String(16))
    quote: Mapped[str] = mapped_column(String(16))
    aliases: Mapped[str] = mapped_column(Text)
    last_price_band: Mapped[str] = mapped_column(String(64))
    adv: Mapped[str] = mapped_column(String(64))


class Store:
    def __init__(self, url: str) -> None:
        self.engine = create_engine(url, future=True)
        Base.metadata.create_all(self.engine)
        self._forbid_tape_updates()

    def _forbid_tape_updates(self) -> None:
        @event.listens_for(self.engine, "before_cursor_execute")
        def _block_update(conn, cursor, statement, parameters, context, executemany):  # type: ignore[no-untyped-def]
            stripped = statement.lstrip().upper()
            if stripped.startswith("UPDATE TAPE_RECORDS") or stripped.startswith("DELETE FROM TAPE_RECORDS"):
                raise RuntimeError("tape_records is append-only")

    def session(self) -> Session:
        return Session(self.engine)


class TapeLogger:
    def __init__(self, store: Store, session_id: str, clock: Callable[[], datetime] | None = None) -> None:
        self.store = store
        self.session_id = session_id
        self._seq = 0
        self._clock = clock or (lambda: datetime.now(timezone.utc))
        with store.session() as db:
            last = db.scalar(
                select(TapeRecord.seq)
                .where(TapeRecord.session_id == session_id)
                .order_by(TapeRecord.seq.desc())
                .limit(1)
            )
            self._seq = int(last or 0)

    def record(self, kind: str, payload: dict[str, Any]) -> None:
        self._seq += 1
        row = TapeRecord(
            session_id=self.session_id,
            seq=self._seq,
            kind=kind,
            ts=self._clock(),
            payload=payload,
        )
        with self.store.session() as db:
            db.add(row)
            db.commit()

    def rows(self) -> list[TapeRecord]:
        with self.store.session() as db:
            return list(
                db.scalars(
                    select(TapeRecord)
                    .where(TapeRecord.session_id == self.session_id)
                    .order_by(TapeRecord.seq.asc())
                )
            )


def replay_text(store: Store, session_id: str) -> str:
    with store.session() as db:
        rows = list(
            db.scalars(
                select(TapeRecord)
                .where(TapeRecord.session_id == session_id)
                .order_by(TapeRecord.seq.asc())
            )
        )
    lines = [f"# tape {session_id} ({len(rows)} events)"]
    for row in rows:
        ts = row.ts.strftime("%H:%M:%S") if row.ts else "??:??:??"
        kind = row.kind
        p = row.payload or {}
        if kind == "stt.final":
            lines.append(f"{ts}  YOU   {p.get('text', '')}")
        elif kind == "cage.reserved":
            lines.append(f"{ts}  RSVD  {p.get('word')}")
        elif kind == "cage.transition":
            lines.append(f"{ts}  CAGE  {p.get('from')} → {p.get('to')} ({p.get('reason')})")
        elif kind == "cage.readback":
            lines.append(f"{ts}  READ  {p.get('text')}")
        elif kind == "tts.speak":
            lines.append(f"{ts}  DESK  {p.get('text')}")
        elif kind == "cage.submit":
            lines.append(f"{ts}  SUB   {p.get('receipt')}")
        elif kind == "cage.fill":
            lines.append(f"{ts}  FILL  {p.get('receipt')}")
        elif kind == "card.push":
            lines.append(f"{ts}  CARD  {p.get('card')} {p.get('state')}")
        elif kind == "latency":
            lines.append(f"{ts}  LAT   {p}")
        else:
            lines.append(f"{ts}  {kind:12} {p}")
    return "\n".join(lines) + "\n"
