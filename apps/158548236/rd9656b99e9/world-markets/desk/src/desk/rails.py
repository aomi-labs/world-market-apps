"""HTTP to the same World rails the Aomi plugin uses: mini-app context, brain, sidecar."""

from __future__ import annotations

import os
from typing import Any
from urllib.parse import quote

import httpx


class WorldRails:
    """Thin HTTP client. No private key; the execution sidecar signs."""

    def __init__(
        self,
        *,
        account_id: int,
        context_url: str,
        brain_url: str,
        execution_url: str,
        bridge_token: str | None = None,
        timeout: float = 30.0,
    ) -> None:
        self.account_id = int(account_id)
        self.context_url = context_url.rstrip("/")
        self.brain_url = brain_url.rstrip("/")
        self.execution_url = execution_url.rstrip("/")
        self.bridge_token = (bridge_token or "").strip() or None
        self.timeout = timeout

    @classmethod
    def from_env(
        cls,
        *,
        account_id: int | None = None,
        context_url: str | None = None,
        brain_url: str | None = None,
        execution_url: str | None = None,
        bridge_token: str | None = None,
    ) -> WorldRails:
        aid = account_id
        if aid is None:
            raw = os.getenv("WORLD_ACCOUNT_ID", "").strip()
            if raw.lower().startswith("world-"):
                raw = raw.split("-", 1)[1]
            if not raw:
                raise RuntimeError(
                    "WORLD_ACCOUNT_ID is required when The Desk is on World rails"
                )
            aid = int(raw)
        return cls(
            account_id=aid,
            context_url=context_url
            or os.getenv("DESK_CONTEXT_URL")
            or os.getenv("MINI_APP_URL")
            or "http://127.0.0.1:8080",
            brain_url=brain_url or os.getenv("WORLD_BRAIN_URL") or "http://127.0.0.1:8788",
            execution_url=execution_url
            or os.getenv("WORLD_EXECUTION_URL")
            or "http://127.0.0.1:8787",
            bridge_token=bridge_token
            if bridge_token is not None
            else os.getenv("DESK_BRIDGE_TOKEN"),
        )

    def context(self) -> dict[str, Any]:
        headers = {}
        if self.bridge_token:
            headers["X-Desk-Token"] = self.bridge_token
        return self._get(f"{self.context_url}/api/v1/desk/context", headers=headers)

    def place_order(self, body: dict[str, Any]) -> dict[str, Any]:
        return self._post(f"{self.execution_url}/v1/orders", body)

    def cancel_order(self, body: dict[str, Any]) -> dict[str, Any]:
        return self._post(f"{self.execution_url}/v1/orders/cancel", body)

    def set_watch(self, body: dict[str, Any]) -> dict[str, Any]:
        payload = {"account_id": self.account_id, **body}
        return self._post(f"{self.brain_url}/v1/watches", payload)

    def cancel_watch(self, watch_id: str) -> dict[str, Any]:
        return self._post(
            f"{self.brain_url}/v1/watches/cancel",
            {"account_id": self.account_id, "id": watch_id},
        )

    def cancel_task(self, task_id: str) -> dict[str, Any]:
        return self._post(
            f"{self.brain_url}/v1/tasks/cancel",
            {"account_id": self.account_id, "id": task_id},
        )

    def tasks(self) -> dict[str, Any]:
        return self._get(f"{self.brain_url}/v1/tasks?account_id={self.account_id}")

    def research(self, symbol: str, window_secs: int = 86400) -> dict[str, Any]:
        sym = quote(symbol, safe="")
        return self._get(
            f"{self.brain_url}/v1/research?symbol={sym}&window_secs={window_secs}"
        )

    def history_move(self, symbol: str, window_secs: int = 86400) -> dict[str, Any]:
        sym = quote(symbol, safe="")
        return self._get(
            f"{self.brain_url}/v1/history/move?symbol={sym}&window_secs={window_secs}"
        )

    def mark_series(self, symbol: str) -> list[dict[str, Any]]:
        sym = quote(symbol, safe="")
        data = self._get(f"{self.brain_url}/v1/history/marks?symbol={sym}")
        marks = data.get("marks") or []
        return marks if isinstance(marks, list) else []

    def _get(self, url: str, *, headers: dict[str, str] | None = None) -> dict[str, Any]:
        try:
            response = httpx.get(url, headers=headers or {}, timeout=self.timeout)
        except httpx.HTTPError as exc:
            raise RuntimeError(_unreachable(url, exc)) from exc
        return _json(response, url)

    def _post(self, url: str, body: dict[str, Any]) -> dict[str, Any]:
        try:
            response = httpx.post(url, json=body, timeout=self.timeout)
        except httpx.HTTPError as exc:
            raise RuntimeError(_unreachable(url, exc)) from exc
        return _json(response, url)


def _json(response: httpx.Response, url: str) -> dict[str, Any]:
    try:
        data = response.json()
    except ValueError as exc:
        raise RuntimeError(f"World rails returned invalid JSON from {url}") from exc
    if not isinstance(data, dict):
        raise RuntimeError(f"World rails returned a non-object from {url}")
    if response.is_error or data.get("ok") is False:
        detail = data.get("error") or data.get("message") or f"HTTP {response.status_code}"
        raise RuntimeError(f"World rails rejected {url}: {detail}")
    return data


def _unreachable(url: str, exc: Exception) -> str:
    return f"World rails are not reachable at {url} ({exc})"
