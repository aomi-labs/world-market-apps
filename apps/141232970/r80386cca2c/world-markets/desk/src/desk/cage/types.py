from __future__ import annotations

from datetime import datetime, timezone
from decimal import Decimal
from enum import StrEnum
from typing import Any, Literal
from uuid import uuid4

from pydantic import BaseModel, Field, field_validator


class CageState(StrEnum):
    IDLE = "IDLE"
    ASSEMBLING = "ASSEMBLING"
    READBACK = "READBACK"
    ARMED_FOR_ASSENT = "ARMED_FOR_ASSENT"
    SUBMITTED = "SUBMITTED"
    WORKING = "WORKING"
    FILLED = "FILLED"
    CANCELLED = "CANCELLED"
    PARKED = "PARKED"


class QuantityKind(StrEnum):
    BASE = "base"
    DOLLARS = "dollars"
    PCT_OF_POSITION = "pct_of_position"


class Quantity(BaseModel):
    kind: QuantityKind
    value: Decimal

    @field_validator("kind", mode="before")
    @classmethod
    def _alias_shares(cls, value: Any) -> Any:
        if isinstance(value, str) and value.lower() in {"shares", "units", "contracts"}:
            return QuantityKind.BASE
        return value

    @field_validator("value", mode="before")
    @classmethod
    def _dec(cls, value: Any) -> Decimal:
        return Decimal(str(value))


class ResolvedInstrument(BaseModel):
    symbol: str
    name: str
    product: Literal["spot", "perp"]
    quote: str = "USDT"
    token_id: int | None = None
    confidence: float
    aliases: list[str] = Field(default_factory=list)
    last_price: Decimal | None = None
    description: str = ""

    @field_validator("symbol")
    @classmethod
    def _upper(cls, value: str) -> str:
        return value.upper()


class OrderDraft(BaseModel):
    id: str = Field(default_factory=lambda: uuid4().hex[:12])
    side: Literal["buy", "sell"] | None = None
    quantity: Quantity | None = None
    instrument_query: str | None = None
    instrument: ResolvedInstrument | None = None
    order_type: Literal["market", "limit", "stop", "stop_limit"] | None = None
    limit_price: Decimal | None = None
    stop_price: Decimal | None = None
    duration: Literal["day", "gtc"] = "day"
    slot_confidence: dict[str, float] = Field(default_factory=dict)

    @field_validator("limit_price", "stop_price", mode="before")
    @classmethod
    def _opt_dec(cls, value: Any) -> Any:
        if value is None or value == "":
            return None
        return Decimal(str(value))

    def merge(self, other: OrderDraft) -> OrderDraft:
        data = self.model_dump()
        incoming = other.model_dump(exclude_unset=True)
        for key, value in incoming.items():
            if key == "id":
                continue
            if key == "slot_confidence":
                merged = dict(data.get("slot_confidence") or {})
                merged.update(value or {})
                data[key] = merged
                continue
            if value is not None:
                data[key] = value
        return OrderDraft.model_validate(data)


class MandateTrigger(BaseModel):
    instrument_query: str = ""
    instrument: ResolvedInstrument | None = None
    comparator: Literal["lt", "gt", "lte", "gte"] = "lt"
    price: Decimal | None = None

    @field_validator("price", mode="before")
    @classmethod
    def _dec(cls, value: Any) -> Any:
        if value is None or value == "":
            return None
        return Decimal(str(value))


class MandateStatus(StrEnum):
    DRAFT = "draft"
    ARMED = "armed"
    SUSPENDED = "suspended"
    FIRED = "fired"
    REVOKED = "revoked"
    EXPIRED = "expired"


class MandateDraft(BaseModel):
    id: str = Field(default_factory=lambda: uuid4().hex[:12])
    name: str | None = None
    trigger: MandateTrigger = Field(default_factory=MandateTrigger)
    action: OrderDraft = Field(default_factory=OrderDraft)
    confirmation_window_sec: int | None = None
    expiry_days: int | None = None
    rationale_text: str | None = None
    rationale_audio_ref: str | None = None
    status: MandateStatus = MandateStatus.DRAFT
    window_started_at: datetime | None = None
    expires_at: datetime | None = None
    suspended_until: datetime | None = None

    def merge(self, other: MandateDraft) -> MandateDraft:
        data = self.model_dump()
        incoming = other.model_dump(exclude_unset=True)
        trig = MandateTrigger.model_validate(data["trigger"])
        other_trig = MandateTrigger.model_validate(incoming.get("trigger") or {})
        if other_trig.instrument_query:
            trig.instrument_query = other_trig.instrument_query
        if other_trig.instrument is not None:
            trig.instrument = other_trig.instrument
        if incoming.get("trigger", {}).get("comparator"):
            trig.comparator = other_trig.comparator
        if other_trig.price is not None:
            trig.price = other_trig.price
        data["trigger"] = trig.model_dump()
        data["action"] = OrderDraft.model_validate(data["action"]).merge(
            OrderDraft.model_validate(incoming.get("action") or {})
        ).model_dump()
        for key in (
            "name",
            "confirmation_window_sec",
            "expiry_days",
            "rationale_text",
            "rationale_audio_ref",
            "status",
            "window_started_at",
            "expires_at",
            "suspended_until",
        ):
            if incoming.get(key) is not None:
                data[key] = incoming[key]
        return MandateDraft.model_validate(data)


class SpokenText(BaseModel):
    text: str
    words: list[str] = Field(default_factory=list)

    @classmethod
    def from_text(cls, text: str) -> SpokenText:
        words = [w for w in text.replace("—", " ").split() if w]
        return cls(text=text, words=words)


class ValidationIssue(BaseModel):
    slot: str | None = None
    code: str
    message: str
    spoken: str


class CardPayload(BaseModel):
    card: Literal["ticket", "book", "registry", "queue", "disambiguation"]
    state: str | None = None
    slots: dict[str, Any] = Field(default_factory=dict)
    confidence: dict[str, float] = Field(default_factory=dict)
    consequence: str | None = None
    ticket_id: str | None = None
    payload: dict[str, Any] = Field(default_factory=dict)


class Quote(BaseModel):
    symbol: str
    name: str
    product: str
    mark: Decimal
    bid: Decimal | None = None
    ask: Decimal | None = None
    as_of: datetime
    source: str

    def age_seconds(self, now: datetime | None = None) -> float:
        now = now or datetime.now(timezone.utc)
        as_of = self.as_of if self.as_of.tzinfo else self.as_of.replace(tzinfo=timezone.utc)
        return max(0.0, (now - as_of).total_seconds())


class Position(BaseModel):
    symbol: str
    product: str
    quantity: Decimal
    avg_price: Decimal | None = None
    mark: Decimal | None = None


class PortfolioSnapshot(BaseModel):
    equity: Decimal
    cash: Decimal
    positions: list[Position] = Field(default_factory=list)
    as_of: datetime = Field(default_factory=lambda: datetime.now(timezone.utc))
