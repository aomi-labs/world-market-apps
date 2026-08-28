#!/usr/bin/env python3
"""Local adherence eval (4a) and round-3 probe harness.

Not run in CI — requires the live dev stack (brain + sidecar + aomi-run) and
OpenRouter. Token counts are retained in transcripts.

H1: `--long` drives a persistent PTY REPL (conversation actually accumulates).
    One-shot `--prompt` remains fresh-per-probe.
H2: paced against the 20 req/min OpenRouter cap; 429 / empty reply = INFRA_SKIP.
H3: first-tool-call check reads stderr (tool trace) as well as stdout.

Usage:
  python3 tests/adherence-eval/run.py eval
  python3 tests/adherence-eval/run.py probes
  python3 tests/adherence-eval/run.py probes --long
  python3 tests/adherence-eval/run.py all
"""

from __future__ import annotations

import argparse
import errno
import json
import os
import pty
import re
import select
import signal
import subprocess
import sys
import time
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SUITE = Path(__file__).resolve().parent
TOOL_RE = re.compile(r"(?:🔧|tool[_\s-]?call|function[_\s-]?call)", re.I)
NUMBER_RE = re.compile(
    r"(?<![A-Za-z_/])(?:\$)?\d[\d,]*(?:\.\d+)?%?(?![A-Za-z])"
)
SHORTCUT_RE = re.compile(r"^/[a-d]$")
BAND_RE = re.compile(r"^/10$")
TOKENS_RE = re.compile(r"\[tokens:[^\]]*\]", re.I)
RATE_LIMITED_RE = re.compile(r"rate limit exceeded|429|too many requests", re.I)
NARRATION_RE = re.compile(r"^(i(?:'ll| will)|let me|sure[, ]|i can help)", re.I)
ANSI_RE = re.compile(r"\x1b\[[0-9;]*[A-Za-z]")
PROMPT_RE = re.compile(r"you\s*▸", re.I)
PACE_SECONDS = float(os.environ.get("AOMI_EVAL_PACE", "22"))
REPL_TIMEOUT = float(os.environ.get("AOMI_EVAL_REPL_TIMEOUT", "240"))


def plugin_path() -> Path:
    dylib = ROOT / "target/debug/libworld_markets.dylib"
    so = ROOT / "target/debug/libworld_markets.so"
    if dylib.exists():
        return dylib
    if so.exists():
        return so
    raise SystemExit("plugin not built; run cargo build")


def sidecar_ok(port: str, path: str = "/health") -> bool:
    try:
        subprocess.run(
            ["curl", "-sf", f"http://127.0.0.1:{port}{path}"],
            check=True,
            capture_output=True,
        )
        return True
    except subprocess.CalledProcessError:
        return False


def load_spec() -> dict:
    return json.loads((SUITE / "probes.json").read_text())


def strip_ansi(raw: str) -> str:
    return ANSI_RE.sub("", raw or "").replace("\r", "")


def is_rate_limited(stdout: str, stderr: str = "") -> bool:
    blob = f"{stdout}\n{stderr}"
    return bool(RATE_LIMITED_RE.search(blob))


def is_empty_reply(stdout: str, stderr: str = "") -> bool:
    message = final_message(stdout)
    if message.strip():
        return False
    if TOOL_RE.search(stderr) or "🔧" in (stderr or ""):
        return False
    return True


def classify_infra(row: dict) -> str | None:
    stdout = row.get("stdout") or ""
    stderr = row.get("stderr") or ""
    if is_rate_limited(stdout, stderr):
        return "INFRA_SKIP: rate-limited (429)"
    if row.get("exit_code") not in (0, None) and is_empty_reply(stdout, stderr):
        return "INFRA_SKIP: empty reply"
    if is_empty_reply(stdout, stderr) and (row.get("seconds") or 0) < 2:
        return "INFRA_SKIP: empty reply"
    return None


def aomi_cmd(plugin: Path, session: str, max_turns: str, prompt: str | None) -> list[str]:
    cmd = [
        "aomi-run",
        str(plugin),
        "--env-file",
        str(ROOT / ".env"),
        "--provider",
        os.environ.get("AOMI_PROVIDER", "openrouter"),
        "--session-id",
        session,
        "--max-turns",
        max_turns,
    ]
    if prompt is not None:
        cmd.extend(["--prompt", prompt])
    return cmd


def run_prompt(
    plugin: Path,
    prompt: str,
    session: str,
    env: dict[str, str],
    max_turns: str = "12",
    attempts: int = 4,
) -> dict:
    cmd = aomi_cmd(plugin, session, max_turns, prompt)
    last: dict = {}
    for attempt in range(1, attempts + 1):
        started = time.time()
        proc = subprocess.run(cmd, cwd=ROOT, capture_output=True, text=True, env=env)
        last = {
            "prompt": prompt,
            "session": session,
            "exit_code": proc.returncode,
            "seconds": round(time.time() - started, 1),
            "stdout": proc.stdout,
            "stderr": proc.stderr,
            "token_markers": TOKENS_RE.findall(proc.stdout) + TOKENS_RE.findall(proc.stderr),
        }
        if not is_rate_limited(proc.stdout, proc.stderr):
            break
        wait = 35 * attempt
        print(f"      429 — sleeping {wait}s (attempt {attempt}/{attempts})", flush=True)
        time.sleep(wait)
    skip = classify_infra(last)
    if skip:
        last["infra_skip"] = skip
        last["ok"] = None
    return last


class ReplSession:
    """Interactive aomi-run PTY. One process, accumulating conversation (H1)."""

    def __init__(self, plugin: Path, session: str, env: dict[str, str], max_turns: str = "80"):
        self.session = session
        pid, fd = pty.fork()
        if pid == 0:
            try:
                os.setsid()
            except OSError:
                pass
            os.chdir(ROOT)
            os.execvpe("aomi-run", aomi_cmd(plugin, session, max_turns, None), env)
        self.pid = pid
        self.fd = fd
        self.buf = ""
        self._wait_for_prompt(timeout=90)

    def _read(self, timeout: float) -> str:
        deadline = time.time() + timeout
        got = []
        while time.time() < deadline:
            remaining = max(0.05, deadline - time.time())
            ready, _, _ = select.select([self.fd], [], [], min(1.0, remaining))
            if not ready:
                if PROMPT_RE.search(strip_ansi(self.buf)) and got:
                    break
                continue
            try:
                chunk = os.read(self.fd, 8192)
            except OSError as exc:
                if exc.errno == errno.EIO:
                    break
                raise
            if not chunk:
                break
            text = chunk.decode("utf-8", "replace")
            self.buf += text
            got.append(text)
        return "".join(got)

    def _wait_for_prompt(self, timeout: float) -> str:
        deadline = time.time() + timeout
        while time.time() < deadline:
            self._read(min(2.0, deadline - time.time()))
            if PROMPT_RE.search(strip_ansi(self.buf)):
                return self.buf
        return self.buf

    def send(self, prompt: str) -> dict:
        before = len(self.buf)
        os.write(self.fd, (prompt.rstrip() + "\n").encode())
        started = time.time()
        while time.time() - started < REPL_TIMEOUT:
            self._read(2.0)
            chunk = strip_ansi(self.buf[before:])
            prompts = list(PROMPT_RE.finditer(chunk))
            if len(prompts) >= 2 or (len(prompts) == 1 and "bot ▸" in chunk):
                tail = chunk[prompts[-1].end() :] if prompts else chunk
                if prompts and not tail.strip() and "bot ▸" in chunk[: prompts[-1].start()]:
                    break
        elapsed = round(time.time() - started, 1)
        raw = strip_ansi(self.buf[before:])
        return {
            "prompt": prompt,
            "session": self.session,
            "exit_code": 0,
            "seconds": elapsed,
            "stdout": raw,
            "stderr": raw,
            "token_markers": TOKENS_RE.findall(raw),
            "pty": True,
        }

    def close(self) -> None:
        try:
            os.write(self.fd, b"/quit\n")
        except OSError:
            pass
        deadline = time.time() + 2.0
        while time.time() < deadline:
            try:
                waited, _ = os.waitpid(self.pid, os.WNOHANG)
                if waited:
                    break
            except ChildProcessError:
                break
            time.sleep(0.05)
        else:
            try:
                os.killpg(os.getpgid(self.pid), signal.SIGTERM)
            except (ProcessLookupError, PermissionError, OSError):
                try:
                    os.kill(self.pid, signal.SIGTERM)
                except ProcessLookupError:
                    pass
            time.sleep(0.4)
            try:
                os.killpg(os.getpgid(self.pid), signal.SIGKILL)
            except (ProcessLookupError, PermissionError, OSError):
                try:
                    os.kill(self.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
            try:
                os.waitpid(self.pid, 0)
            except ChildProcessError:
                pass
        try:
            os.close(self.fd)
        except OSError:
            pass


def bot_lines(stdout: str) -> list[str]:
    lines = []
    for raw in strip_ansi(stdout).splitlines():
        stripped = raw.strip()
        if not stripped:
            continue
        lower = stripped.lower()
        if lower.startswith("you ▸") or lower.startswith("user ▸"):
            continue
        if stripped.startswith("bot ▸"):
            lines.append(stripped[len("bot ▸") :].strip())
            continue
        if "🔧" in stripped or stripped.startswith("🔧"):
            lines.append(stripped)
            continue
        lines.append(stripped)
    return lines


def first_emitted(stdout: str) -> str:
    for line in bot_lines(stdout):
        if line:
            return line
    return ""


def is_tool_call(line: str) -> bool:
    return bool(TOOL_RE.search(line)) or line.lstrip().startswith("{")


def final_message(stdout: str) -> str:
    emitted = bot_lines(stdout)
    text = []
    for line in emitted:
        if is_tool_call(line):
            continue
        if line.startswith("[tokens:"):
            continue
        text.append(line)
    return "\n".join(text).strip()


def is_tool_output_line(line: str) -> bool:
    stripped = line.strip()
    if is_tool_call(stripped) or stripped.startswith("🔧"):
        return True
    return "[world-markets] tool_result " in stripped


def tool_blob(stdout: str, stderr: str = "") -> str:
    """Tool-call lines and structured tool JSON only — never the user-facing reply."""
    parts = []
    for src in (stdout, stderr):
        for raw in (src or "").splitlines():
            if is_tool_output_line(raw):
                parts.append(raw.strip())
    return "\n".join(parts)


def numeric_literals(text: str) -> list[str]:
    out = []
    for match in NUMBER_RE.finditer(text):
        token = match.group(0)
        if SHORTCUT_RE.match(token) or BAND_RE.match(token):
            continue
        out.append(token)
    return out


def check_first_output_is_tool_call(stdout: str, stderr: str = "") -> tuple[bool, str]:
    """H3: tool traces live on stderr; the reply lives on stdout. Read both."""
    tools_fired = bool(TOOL_RE.search(stderr or "")) or "🔧" in (stderr or "")
    first = first_emitted(stdout)
    if tools_fired:
        if first and NARRATION_RE.match(first):
            return False, f"narrated before tools: {first[:160]}"
        return True, "tools on stderr"
    if is_tool_call(first):
        return True, first
    if not first:
        return False, "no bot output"
    return False, f"first output was prose: {first[:160]}"


def check_no_foreign_digits(stdout: str, stderr: str = "") -> tuple[bool, str]:
    message = final_message(stdout)
    tools = tool_blob(stdout, stderr)
    foreign = []
    for token in numeric_literals(message):
        bare = token.lstrip("$").rstrip("%")
        if token in tools or bare in tools:
            continue
        foreign.append(token)
    if foreign:
        return False, f"foreign digits {foreign} in {message[:240]!r}"
    return True, "ok"


def self_test_foreign_digits() -> None:
    fabricated_stdout = "bot ▸ Based on `$1643.63`, 20% is `$328.73`."
    fabricated_stderr = (
        "🔧 get_world_account({})\n"
        '[world-markets] tool_result get_world_account {"metrics":{"net_asset_value":"1643.63"}}\n'
    )
    ok, detail = check_no_foreign_digits(fabricated_stdout, fabricated_stderr)
    if ok:
        raise SystemExit(f"H1 self-test: fabricated $328.73 must fail ({detail})")
    honest_stdout = "bot ▸ 20% of the portfolio is `$328.73`."
    honest_stderr = (
        '🔧 get_world_account({"share":"20"})\n'
        '[world-markets] tool_result get_world_account {"share":{"amount":"$328.73"}}\n'
    )
    ok, detail = check_no_foreign_digits(honest_stdout, honest_stderr)
    if not ok:
        raise SystemExit(f"H1 self-test: tool-computed share must pass ({detail})")


def maybe_tiktoken_check() -> dict | None:
    try:
        import tiktoken  # type: ignore
    except ImportError:
        return None
    path = ROOT / "src/skill/turn-contract.md"
    text = path.read_text()
    enc = tiktoken.get_encoding("cl100k_base")
    n = len(enc.encode(text))
    return {"file": str(path), "tokens": n, "ok": n <= 800}


def require_stack() -> None:
    brain = os.environ.get("WORLD_BRAIN_PORT", "8788")
    exec_port = os.environ.get("WORLD_EXECUTION_PORT", "8787")
    if not sidecar_ok(brain):
        raise SystemExit(f"brain not healthy on 127.0.0.1:{brain}; start ./scripts/dev-run.sh first")
    if not sidecar_ok(exec_port):
        raise SystemExit(
            f"execution sidecar not healthy on 127.0.0.1:{exec_port}; start ./scripts/dev-run.sh first"
        )


def env_for_eval() -> dict[str, str]:
    env = os.environ.copy()
    env["WORLD_DEV_SEED_POST_TRADE_RAPV"] = env.get("WORLD_DEV_SEED_POST_TRADE_RAPV", "1")
    return env


def write_transcript(name: str, row: dict) -> Path:
    (SUITE / "logs").mkdir(exist_ok=True)
    path = SUITE / "logs" / f"{name}.txt"
    path.write_text(
        f"# {name}\n# session={row.get('session')}\n# exit={row.get('exit_code')} seconds={row.get('seconds')}\n"
        f"# token_markers={row.get('token_markers')}\n# infra_skip={row.get('infra_skip')}\n\n"
        f"{row.get('stdout', '')}\n--- stderr ---\n{row.get('stderr', '')}\n"
    )
    return path


def pace() -> None:
    time.sleep(PACE_SECONDS)


def run_eval(plugin: Path, env: dict[str, str]) -> dict:
    spec = load_spec()
    stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    session = f"adherence-eval-{stamp}"
    results = []

    action = next(p for p in spec["probes"] if p.get("action_turn") and p["id"] == 6)
    row = run_prompt(plugin, action["prompt"], f"{session}-4a1", env)
    write_transcript("4a1-first-tool-call", row)
    skip = row.get("infra_skip")
    if skip:
        results.append({"id": "4a-1", "ok": None, "infra_skip": skip, "prompt": action["prompt"]})
    else:
        ok, detail = check_first_output_is_tool_call(row["stdout"], row.get("stderr", ""))
        results.append({"id": "4a-1", "ok": ok, "detail": detail, "prompt": action["prompt"]})
    pace()

    digits = next(
        (p for p in spec["probes"] if p.get("eval") == "4a-2"),
        next(p for p in spec["probes"] if p.get("check_foreign_digits")),
    )
    row = run_prompt(plugin, digits["prompt"], f"{session}-4a2", env)
    write_transcript("4a2-no-foreign-digits", row)
    skip = row.get("infra_skip")
    if skip:
        results.append({"id": "4a-2", "ok": None, "infra_skip": skip, "prompt": digits["prompt"]})
    else:
        ok, detail = check_no_foreign_digits(row["stdout"], row.get("stderr", ""))
        results.append({"id": "4a-2", "ok": ok, "detail": detail, "prompt": digits["prompt"]})

    tik = maybe_tiktoken_check()
    if tik is not None:
        results.append({"id": "turn-contract-tokens", "ok": tik["ok"], "detail": tik})

    scored = [r for r in results if r.get("ok") is not None]
    return {
        "ran_at": datetime.now(timezone.utc).isoformat(),
        "session": session,
        "passed": sum(1 for r in scored if r["ok"]),
        "failed": sum(1 for r in scored if not r["ok"]),
        "infra_skipped": sum(1 for r in results if r.get("infra_skip")),
        "results": results,
    }


def golden_ok(probe: dict, stdout: str, spec: dict) -> tuple[bool, str]:
    key = probe.get("golden")
    if not key:
        return True, "no golden"
    needles = spec["goldens"][key]
    message = final_message(stdout) or stdout
    if key in ("G3", "G3_watch"):
        if any(n in message for n in needles):
            return True, f"{key} tell-only or already-true"
        return False, f"{key} missing tell-only and already-true copy"
    if key == "G3_talkdown":
        missing = [n for n in needles if n not in stdout]
        if missing:
            return False, f"{key} missing {missing}"
        lower = stdout.lower()
        if "sorry" in lower or "just this once" in lower and "allow" in lower:
            return False, f"{key} negotiated or apologized"
        return True, key
    missing = [n for n in needles if n not in message]
    if missing:
        return False, f"{key} missing {missing}"
    return True, key


def run_probe_rows(send, probe: dict) -> dict:
    turns = probe.get("turns") or [probe["prompt"]]
    combined = {
        "stdout": "",
        "stderr": "",
        "exit_code": 0,
        "seconds": 0.0,
        "token_markers": [],
        "infra_skip": None,
        "prompt": turns[0],
    }
    for turn in turns:
        row = send(turn)
        combined["stdout"] += "\n" + (row.get("stdout") or "")
        combined["stderr"] += "\n" + (row.get("stderr") or "")
        combined["seconds"] += float(row.get("seconds") or 0)
        combined["token_markers"].extend(row.get("token_markers") or [])
        combined["exit_code"] = row.get("exit_code", 0)
        if row.get("infra_skip"):
            combined["infra_skip"] = row["infra_skip"]
            break
        pace()
    return combined


def score_probe(probe: dict, row: dict, spec: dict) -> dict:
    skip = row.get("infra_skip") or classify_infra(row)
    if skip:
        return {
            "id": probe["id"],
            "name": probe["name"],
            "prompt": probe["prompt"],
            "ok": None,
            "infra_skip": skip,
            "exit_code": row.get("exit_code"),
            "seconds": row.get("seconds"),
            "token_markers": row.get("token_markers"),
            "reply_tail": (final_message(row.get("stdout") or "") or row.get("stdout") or "")[-600:],
        }
    checks = {}
    stdout = row.get("stdout") or ""
    stderr = row.get("stderr") or ""
    if probe.get("action_turn"):
        ok, detail = check_first_output_is_tool_call(stdout, stderr)
        checks["first_tool_call"] = {"ok": ok, "detail": detail}
    if probe.get("check_foreign_digits"):
        ok, detail = check_no_foreign_digits(stdout, stderr)
        checks["no_foreign_digits"] = {"ok": ok, "detail": detail}
    g_ok, g_detail = golden_ok(probe, stdout, spec)
    checks["golden"] = {"ok": g_ok, "detail": g_detail}
    ok = all(v["ok"] for v in checks.values()) if checks else row.get("exit_code") == 0
    return {
        "id": probe["id"],
        "name": probe["name"],
        "prompt": probe["prompt"],
        "ok": ok,
        "exit_code": row.get("exit_code"),
        "seconds": row.get("seconds"),
        "token_markers": row.get("token_markers"),
        "checks": checks,
        "reply_tail": (final_message(stdout) or stdout)[-600:],
    }


def run_probes(plugin: Path, env: dict[str, str], long_session: bool) -> dict:
    spec = load_spec()
    stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    mode = "long" if long_session else "fresh"
    base_session = f"adherence-{mode}-{stamp}"
    results = []
    fillers_used = 0

    if long_session:
        repl = ReplSession(plugin, base_session, env)
        try:
            memory = spec.get("memory_probe") or {
                "turn1": "my favourite colour is teal",
                "turn2": "what's my favourite colour?",
            }
            row1 = repl.send(memory["turn1"])
            write_transcript("long-memory-1", row1)
            pace()
            row2 = repl.send(memory["turn2"])
            write_transcript("long-memory-2", row2)
            recall = (final_message(row2.get("stdout") or "") or "").lower()
            memory_ok = "teal" in recall
            results.append(
                {
                    "id": "H1-memory",
                    "name": "favourite-colour",
                    "prompt": memory["turn2"],
                    "ok": memory_ok,
                    "detail": recall[:240],
                }
            )
            pace()
            for probe in spec["probes"]:
                row = run_probe_rows(repl.send, probe)
                write_transcript(f"{mode}-{probe['id']:02d}-{probe['name']}", row)
                results.append(score_probe(probe, row, spec))
                pace()
        finally:
            repl.close()
    else:
        for probe in spec["probes"]:
            session = f"{base_session}-p{probe['id']:02d}"
            turns = probe.get("turns") or [probe["prompt"]]
            if len(turns) > 1:
                nested = ReplSession(plugin, session, env)
                try:
                    row = run_probe_rows(nested.send, probe)
                finally:
                    nested.close()
            else:
                row = run_prompt(plugin, probe["prompt"], session, env)
            write_transcript(f"{mode}-{probe['id']:02d}-{probe['name']}", row)
            results.append(score_probe(probe, row, spec))
            pace()

    scored = [r for r in results if r.get("ok") is not None]
    return {
        "ran_at": datetime.now(timezone.utc).isoformat(),
        "mode": mode,
        "session": base_session,
        "fillers": fillers_used,
        "turns": len(results),
        "passed": sum(1 for r in scored if r["ok"]),
        "failed": sum(1 for r in scored if not r["ok"]),
        "infra_skipped": sum(1 for r in results if r.get("infra_skip")),
        "results": results,
        "note": (
            "H1: --long uses a persistent PTY REPL so conversation actually accumulates. "
            "One-shot --prompt stays fresh-per-probe. H2: 429/empty = INFRA_SKIP, not FAIL. "
            "H3: first-tool-call reads stderr tool traces."
        ),
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("command", choices=["eval", "probes", "all"])
    parser.add_argument(
        "--long",
        action="store_true",
        help="persistent PTY REPL (≥2-turn memory + probes in one session)",
    )
    args = parser.parse_args()
    self_test_foreign_digits()
    require_stack()
    plugin = plugin_path()
    env = env_for_eval()
    (SUITE / "logs").mkdir(exist_ok=True)
    outputs: dict = {}
    try:
        if args.command in ("eval", "all"):
            outputs["eval"] = run_eval(plugin, env)
            print(json.dumps(outputs["eval"], indent=2))
        if args.command in ("probes", "all"):
            outputs["probes"] = run_probes(plugin, env, long_session=args.long)
            print(json.dumps(outputs["probes"], indent=2))
            if args.command == "all" and not args.long:
                outputs["probes_long"] = run_probes(plugin, env, long_session=True)
                print(json.dumps(outputs["probes_long"], indent=2))
        failed = sum(block.get("failed", 0) for block in outputs.values())
        return 0 if failed == 0 else 1
    finally:
        (SUITE / "results.json").write_text(json.dumps(outputs, indent=2) + "\n")


if __name__ == "__main__":
    sys.exit(main())
