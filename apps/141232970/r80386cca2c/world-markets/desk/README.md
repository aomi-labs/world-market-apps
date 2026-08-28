# The Desk

Leftover Vite/LiveKit rehearsal client. **Production voice-in is the Mini App
hold-to-talk + Telegram chat**, into the Aomi plugin. This process is not the
money gate.

## Local loop

```sh
cd desk
python3 -m venv .venv
source .venv/bin/activate
pip install -e ".[dev]"
pytest
```

`python -m desk serve` still boots the Cage room for tape replay. Mini App
hold-to-talk does **not** post here.

Production voice:

```
Mini App 🎙 → POST /api/v1/mini-app/voice → STT → brain utterance
            → sendData(transcript) → Aomi agent → execute_* (sidecar signs)
```

Assent for orders is voice or text in the thread, not Cage `Done`, and not a
Telegram Sign button.
