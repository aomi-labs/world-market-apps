from __future__ import annotations

import os
from decimal import Decimal
from pathlib import Path
from typing import Any, Literal

import yaml
from pydantic import BaseModel, ConfigDict, Field


class DeskConfig(BaseModel):
    model_config = ConfigDict(extra="ignore")
    verbosity: Literal["novice", "expert"] = "expert"
    quantity_cap_pct_of_equity: Decimal = Decimal("0.25")
    quiet_hours: list[str] | None = None
    anchor_time: str = "09:30"
    voice_id: str = "cartesia:sonic-3.6:desk-v0-pinned"
    eot_threshold: float = 0.72
    instrument_confidence_threshold: float = 0.9
    assent_timeout_sec: float = 30
    mandate_confirmation_window_sec: int = 300
    mandate_expiry_days: int = 30
    mandate_poll_sec: float = 5
    limit_offset_bps: Decimal = Decimal("10")
    default_product: Literal["spot", "perp"] = "spot"
    default_quote: str = "USDT"
    watchlist: list[str] = Field(default_factory=lambda: ["WETH", "WBTC"])
    aomi_mandate_path: str = "placeholder"
    world_rpc_url: str | None = None
    world_execution_url: str | None = None
    world_brain_url: str | None = None
    desk_context_url: str | None = None
    desk_bridge_token: str | None = None
    world_account_id: int | None = None
    bind: str = "127.0.0.1:8765"
    data_dir: Path = Path("data")


def _decimalish(value: Any) -> Any:
    if isinstance(value, dict):
        return {k: _decimalish(v) for k, v in value.items()}
    if isinstance(value, list):
        return [_decimalish(v) for v in value]
    return value


def _parse_account_id(raw: str | None) -> int | None:
    if not raw:
        return None
    trimmed = raw.strip()
    if trimmed.lower().startswith("world-"):
        trimmed = trimmed.split("-", 1)[1]
    if not trimmed:
        return None
    try:
        return int(trimmed)
    except ValueError:
        return None


def load_config(path: Path | None = None) -> DeskConfig:
    desk_root = Path(__file__).resolve().parents[2]
    path = path or desk_root / "desk_config.yaml"
    raw: dict[str, Any] = {}
    if path.exists():
        loaded = yaml.safe_load(path.read_text()) or {}
        raw = _decimalish(loaded)
    cfg = DeskConfig.model_validate(raw)
    if os.getenv("WORLD_EXECUTION_URL"):
        cfg.world_execution_url = os.getenv("WORLD_EXECUTION_URL")
    if os.getenv("WORLD_BRAIN_URL"):
        cfg.world_brain_url = os.getenv("WORLD_BRAIN_URL")
    if os.getenv("DESK_CONTEXT_URL") or os.getenv("MINI_APP_URL"):
        cfg.desk_context_url = os.getenv("DESK_CONTEXT_URL") or os.getenv("MINI_APP_URL")
    if os.getenv("DESK_BRIDGE_TOKEN"):
        cfg.desk_bridge_token = os.getenv("DESK_BRIDGE_TOKEN")
    account = _parse_account_id(os.getenv("WORLD_ACCOUNT_ID"))
    if account is not None:
        cfg.world_account_id = account
    bind = os.getenv("DESK_BIND")
    if bind:
        cfg.bind = bind
    data_dir = os.getenv("DESK_DATA_DIR")
    if data_dir:
        cfg.data_dir = Path(data_dir)
    mandate = os.getenv("WORLD_MANDATE_PATH")
    if mandate:
        cfg.aomi_mandate_path = mandate
    return cfg
