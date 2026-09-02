"""Live vendor pipeline. Dark until LIVEKIT_* / DEEPGRAM / CARTESIA / ANTHROPIC keys exist."""

from __future__ import annotations

import os


def voice_vendors_configured() -> bool:
    required = (
        "LIVEKIT_URL",
        "LIVEKIT_API_KEY",
        "LIVEKIT_API_SECRET",
        "DEEPGRAM_API_KEY",
        "ANTHROPIC_API_KEY",
        "CARTESIA_API_KEY",
    )
    return all(os.getenv(k) for k in required)


def build_livekit_agent():
    """Import livekit-agents only when extras and keys are present."""
    if not voice_vendors_configured():
        raise RuntimeError(
            "Voice vendors are not configured. Run the mock room: python -m desk serve"
        )
    try:
        from livekit.agents import Agent, AgentSession  # type: ignore
    except ImportError as exc:
        raise RuntimeError('Install voice extras: pip install -e ".[voice]"') from exc
    _ = Agent, AgentSession
    raise RuntimeError(
        "LiveKit worker wiring is the M1 vendor step. The Cage, tape, and cards already run "
        "through the local WebSocket room; point this worker at DeskSession.on_final_transcript "
        "and notify_tts_complete when keys land."
    )
