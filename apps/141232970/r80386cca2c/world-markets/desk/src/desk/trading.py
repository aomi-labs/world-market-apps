from __future__ import annotations

from typing import Any

from desk.config import DeskConfig


def make_broker(config: DeskConfig) -> Any:
    from desk.world_broker import WorldBroker

    return WorldBroker.from_config(config)
