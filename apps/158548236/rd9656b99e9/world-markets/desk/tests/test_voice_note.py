from fastapi.testclient import TestClient

from desk.config import DeskConfig
from desk.persist import Store
from desk.server import create_app
from desk.stt import SttError, transcribe, _content_type
from stub_broker import StubBroker


def _client(tmp_path):
    cfg = DeskConfig(aomi_mandate_path="placeholder")
    store = Store(f"sqlite:///{tmp_path / 'desk.sqlite'}")
    return TestClient(create_app(cfg, store=store, broker=StubBroker()))


def test_voice_note_text_uses_cage(tmp_path):
    client = _client(tmp_path)
    sid = client.post("/api/session").json()["session_id"]
    out = client.post("/api/voice/note", json={"session_id": sid, "text": "positions"})
    assert out.status_code == 200
    body = out.json()
    assert body["transcript"] == "positions"
    assert "World book" in (body.get("speech") or "")


def test_voice_note_requires_audio_or_text(tmp_path):
    client = _client(tmp_path)
    res = client.post("/api/voice/note", json={"session_id": "mini"})
    assert res.status_code == 400


def test_health_reports_rails(tmp_path):
    client = _client(tmp_path)
    health = client.get("/api/health").json()
    assert health["ok"] is True
    assert health["rails"] is True
    assert "paper_mode" not in health


def test_transcribe_without_keys(monkeypatch):
    monkeypatch.delenv("DEEPGRAM_API_KEY", raising=False)
    monkeypatch.delenv("OPENAI_API_KEY", raising=False)
    try:
        transcribe(b"abc", "audio/webm")
        raise AssertionError
    except SttError as exc:
        assert "not configured" in str(exc)


def test_content_type_sniffs_wav_over_a_webm_label():
    wav = b"RIFF\x00\x00\x00\x00WAVE" + b"\x00" * 8
    assert _content_type(wav, "audio/webm") == "audio/wav"
    assert _content_type(b"OggS....", None) == "audio/ogg"
    assert _content_type(b"???", "audio/mp4") == "audio/mp4"
