"""Typed tool surface the Brain may call. None of these submit an order."""

from __future__ import annotations

from typing import Any, Literal

from pydantic import BaseModel, Field


class GetQuoteArgs(BaseModel):
    instrument: str


class SearchInstrumentArgs(BaseModel):
    query: str
    product: Literal["spot", "perp"] | None = None


class ProposeOrderArgs(BaseModel):
    side: Literal["buy", "sell"] | None = None
    quantity_kind: Literal["base", "dollars", "pct_of_position"] | None = None
    quantity_value: str | None = None
    instrument_query: str | None = None
    order_type: Literal["market", "limit", "stop", "stop_limit"] | None = None
    limit_price: str | None = None
    stop_price: str | None = None
    duration: Literal["day", "gtc"] | None = None
    slot_confidence: dict[str, float] = Field(default_factory=dict)


class ProposeMandateArgs(BaseModel):
    instrument_query: str
    comparator: Literal["lt", "gt", "lte", "gte"] = "lt"
    price: str
    side: Literal["buy", "sell"] = "sell"
    quantity_kind: Literal["base", "dollars", "pct_of_position"] = "pct_of_position"
    quantity_value: str = "0.5"


class SendCardArgs(BaseModel):
    card_type: Literal["ticket", "book", "registry", "queue", "disambiguation"]
    payload: dict[str, Any] = Field(default_factory=dict)


TOOL_NAMES = (
    "get_quote",
    "get_positions",
    "get_portfolio_summary",
    "search_instrument",
    "propose_order",
    "propose_mandate",
    "run_mandate_simulation",
    "list_mandates",
    "suspend_mandate",
    "revoke_mandate",
    "add_to_watchlist",
    "remove_from_watchlist",
    "journal_note",
    "send_card",
    "generate_open",
)

# Deliberately absent: submit_order, execute_order, place_order.
