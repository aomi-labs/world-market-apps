from __future__ import annotations

import asyncio
import base64
import os
from pathlib import Path
from typing import Any
from uuid import uuid4

from fastapi import FastAPI, Header, HTTPException, WebSocket, WebSocketDisconnect
from fastapi.middleware.cors import CORSMiddleware
from fastapi.staticfiles import StaticFiles

from desk.config import DeskConfig, load_config
from desk.persist import Store, TapeLogger, replay_text
from desk.session import DeskSession
from desk.stt import SttError, transcribe
from desk.voice import voice_vendors_configured

ROOT = Path(__file__).resolve().parents[2]
ASSETS = ROOT / "assets"


def create_app(
    config: DeskConfig | None = None,
    *,
    store: Store | None = None,
    broker: Any | None = None,
) -> FastAPI:
    config = config or load_config()
    data_dir = Path(config.data_dir)
    data_dir.mkdir(parents=True, exist_ok=True)
    store = store or Store(f"sqlite:///{data_dir / 'desk.sqlite'}")
    app = FastAPI(title="The Desk")
    app.add_middleware(
        CORSMiddleware,
        allow_origins=["*"],
        allow_methods=["*"],
        allow_headers=["*"],
    )
    sessions: dict[str, DeskSession] = {}
    expected_token = (config.desk_bridge_token or os.getenv("DESK_BRIDGE_TOKEN") or "").strip()

    def get_session(session_id: str | None = None) -> DeskSession:
        sid = session_id or uuid4().hex[:10]
        if sid not in sessions:
            tape = TapeLogger(store, sid)
            tape.record("session.start", {"config": config.model_dump(mode="json")})
            sessions[sid] = DeskSession(config, tape=tape, broker=broker)
        return sessions[sid]

    def require_bridge(x_desk_token: str | None) -> None:
        if not expected_token:
            return
        if (x_desk_token or "").strip() != expected_token:
            raise HTTPException(status_code=401, detail="desk token required")

    @app.get("/api/health")
    def health() -> dict[str, Any]:
        return {
            "ok": True,
            "rails": True,
            "voice_vendors": voice_vendors_configured(),
        }

    @app.get("/api/token")
    def token() -> dict[str, Any]:
        return {"ok": False, "error": "LiveKit keys not configured; using local mock room"}

    @app.post("/api/session")
    def new_session() -> dict[str, str]:
        s = get_session()
        return {"session_id": s.session_id}

    def run_turn(session: DeskSession, text: str, *, complete_tts: bool = True) -> dict[str, Any]:
        out = session.on_final_transcript(text)
        if complete_tts and session.interrupt.playing:
            out["tts_complete"] = session.notify_tts_complete()
        return out

    @app.post("/api/inject/{session_id}")
    def inject(session_id: str, body: dict[str, Any]) -> dict[str, Any]:
        s = get_session(session_id)
        text = body.get("text") or ""
        return run_turn(s, text, complete_tts=body.get("complete_tts", True))

    @app.post("/api/voice/note")
    def voice_note(
        body: dict[str, Any],
        x_desk_token: str | None = Header(default=None),
    ) -> dict[str, Any]:
        require_bridge(x_desk_token)
        session_id = str(body.get("session_id") or "mini")
        s = get_session(session_id)
        text = str(body.get("text") or "").strip()
        if not text:
            raw_b64 = body.get("audio_base64") or body.get("audio")
            if not raw_b64:
                raise HTTPException(status_code=400, detail="audio or text is required")
            try:
                audio = base64.b64decode(raw_b64)
            except Exception as exc:  # noqa: BLE001
                raise HTTPException(status_code=400, detail="audio is not valid base64") from exc
            try:
                text = transcribe(audio, body.get("mime"))
            except SttError as exc:
                raise HTTPException(status_code=502, detail=str(exc)) from exc
        try:
            out = run_turn(s, text, complete_tts=body.get("complete_tts", True))
        except Exception as exc:  # noqa: BLE001
            raise HTTPException(status_code=502, detail=str(exc)) from exc
        out["transcript"] = text
        out["session_id"] = s.session_id
        return out

    @app.get("/api/tape/{session_id}")
    def tape(session_id: str) -> dict[str, str]:
        return {"text": replay_text(store, session_id)}

    @app.get("/api/latency/{session_id}")
    def latency(session_id: str) -> dict[str, str]:
        s = sessions.get(session_id)
        return {"report": s.latency_report() if s else "no session"}

    @app.websocket("/ws")
    async def ws(websocket: WebSocket) -> None:
        await websocket.accept()
        session: DeskSession | None = None

        def push(msg: dict[str, Any]) -> None:
            try:
                loop = asyncio.get_running_loop()
            except RuntimeError:
                return
            loop.create_task(websocket.send_json(msg))

        try:
            while True:
                data = await websocket.receive_json()
                typ = data.get("type")
                if typ == "hello":
                    session = get_session(data.get("session_id"))
                    session.push = push
                    await websocket.send_json(
                        {
                            "type": "hello",
                            "session_id": session.session_id,
                            "rails": True,
                        }
                    )
                    continue
                if session is None:
                    session = get_session()
                    session.push = push
                if typ == "transcript":
                    out = session.on_final_transcript(data.get("text") or "")
                    await websocket.send_json({"type": "turn", **out})
                    if data.get("auto_complete_tts", True) and session.interrupt.playing:
                        done = session.notify_tts_complete()
                        await websocket.send_json({"type": "tts_complete", **done})
                elif typ == "tts_complete":
                    await websocket.send_json({"type": "tts_complete", **session.notify_tts_complete()})
                elif typ == "tick":
                    for item in session.tick():
                        await websocket.send_json({"type": "tick", **item})
        except WebSocketDisconnect:
            return

    ear = ASSETS / "earcons"
    if ear.exists():
        app.mount("/earcons", StaticFiles(directory=ear), name="earcons")
    dist = ROOT / "client" / "dist"
    if dist.exists():
        app.mount("/", StaticFiles(directory=dist, html=True), name="ui")
    else:

        @app.get("/")
        def root() -> dict[str, Any]:
            return {"ok": True, "hint": "run the Vite client in desk/client"}

    return app


def serve(config: DeskConfig) -> None:
    import uvicorn

    host, _, port = config.bind.partition(":")
    uvicorn.run(create_app(config), host=host or "127.0.0.1", port=int(port or 8765), log_level="info")
