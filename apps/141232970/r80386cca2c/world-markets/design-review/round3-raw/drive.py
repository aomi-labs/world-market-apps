#!/usr/bin/env python3
"""Round-3 probe driver — paced for the 20 RPM cap on anthropic/claude-4-sonnet.

VERIFIED CONSTRAINTS (do not re-discover these):
1. `--session-id` does NOT persist conversation history across separate
   `aomi-run --prompt` invocations. Turn 1 and turn 2 of the same session id both
   opened at in=26,870 and the agent had no recall of a fact stated in turn 1.
   => every --prompt call is a fresh turn-1 session.
   => the eval harness's `--long` mode is NOT a long session; it is 20 independent
      turn-1 sessions. Its `fillers` do nothing.
   => sequential arcs (graduation, ledger recall, correction) MUST run in the
      interactive PTY REPL.
2. OpenRouter caps this account at 20 requests/min for claude-4-sonnet. One probe
   turn = 2-5 LLM calls. Pace accordingly or every later probe returns exit 1 with
   an empty reply that looks like an agent defect but is a 429.
"""
from __future__ import annotations
import json, os, re, subprocess, sys, time
from pathlib import Path

ROOT = Path("/Users/lucas/Desktop/World/aomi")
OUT = ROOT / "design-review" / "round3-raw"
OUT.mkdir(parents=True, exist_ok=True)

BANNER_END = "stubbed 12 tools for namespace 'evm-core'"
ANSI = re.compile(r"\x1b\[[0-9;]*[A-Za-z]")
TOKENS = re.compile(r"\[tokens: in=(\d+) out=(\d+) total=(\d+)\]")
RATE_LIMITED = "Rate limit exceeded"
PACE_SECONDS = float(os.environ.get("R3_PACE", "22"))


def clean(raw: str) -> str:
    txt = ANSI.sub("", raw).replace("\r", "")
    if BANNER_END in txt:
        txt = txt.split(BANNER_END, 1)[1]
    return txt.strip()


def strip_noise(body: str) -> str:
    keep = []
    for line in body.splitlines():
        if "rig::providers" in line or "rig::agent" in line:
            continue
        if line.strip().startswith("Caused by:"):
            continue
        if RATE_LIMITED in line:
            continue
        keep.append(line)
    return "\n".join(keep).strip()


def one_shot(prompt: str, session: str, max_turns: int, timeout: int) -> tuple[str, str, int]:
    cmd = [
        "aomi-run", "target/debug/libworld_markets.dylib",
        "--env-file", ".env", "--provider", "openrouter",
        "--session-id", session, "--max-turns", str(max_turns),
        "--prompt", prompt,
    ]
    e = os.environ.copy()
    e["WORLD_DEV_SEED_POST_TRADE_RAPV"] = "1"
    try:
        p = subprocess.run(cmd, cwd=ROOT, capture_output=True, text=True,
                           env=e, timeout=timeout)
        return p.stdout, p.stderr, p.returncode
    except subprocess.TimeoutExpired as ex:
        so = ex.stdout.decode() if isinstance(ex.stdout, bytes) else (ex.stdout or "")
        se = ex.stderr.decode() if isinstance(ex.stderr, bytes) else (ex.stderr or "")
        return so, se + "\n[TIMEOUT]", -1


def run(prompt: str, session: str, max_turns: int = 20, timeout: int = 300,
        attempts: int = 4) -> dict:
    t0 = time.time()
    so, se, code = "", "", -99
    for attempt in range(1, attempts + 1):
        so, se, code = one_shot(prompt, session, max_turns, timeout)
        if RATE_LIMITED not in (so + se):
            break
        wait = 35 * attempt
        print(f"      429 — sleeping {wait}s (attempt {attempt}/{attempts})", flush=True)
        time.sleep(wait)
    # aomi-run: the agent's REPLY goes to stdout; the tool trace + banner to stderr.
    # Never concatenate before splitting on the banner or the reply is discarded.
    reply_raw = ANSI.sub("", so).replace("\r", "")
    trace = clean(se)
    m = TOKENS.search(reply_raw + trace)
    tools = re.findall(r"🔧 (\w+)\(", trace)
    tool_args = re.findall(r"🔧 (\w+\([^\n]*)", trace)
    tool_errors = re.findall(r"plugin tool error: \[world-markets\] ([^\n]+)", trace)
    reply = strip_noise(TOKENS.sub("", reply_raw))
    reply = re.sub(r"^bot ▸\s*", "", reply, flags=re.M)
    reply = re.sub(r"\n{3,}", "\n\n", reply).strip()
    return {
        "prompt": prompt, "session": session, "exit": code,
        "seconds": round(time.time() - t0, 1),
        "tokens_in": int(m.group(1)) if m else None,
        "tokens_out": int(m.group(2)) if m else None,
        "rate_limited": RATE_LIMITED in (so + se),
        "tools": tools, "tool_args": tool_args, "tool_errors": tool_errors,
        "reply": reply, "trace": trace,
    }


def main() -> int:
    spec = json.loads(Path(sys.argv[1]).read_text())
    tag = spec["tag"]
    dest = OUT / f"{tag}.json"
    rows = json.loads(dest.read_text()) if dest.exists() else []
    done = {r["id"] for r in rows}
    todo = [p for p in spec["probes"] if p.get("id") not in done]
    print(f"{tag}: {len(todo)} to run, {len(done)} already captured", flush=True)
    for i, item in enumerate(todo, 1):
        pid = item["id"]
        sess = f"r3-{tag}-{pid}"
        print(f"[{i}/{len(todo)}] {pid}: {item['prompt'][:70]}", flush=True)
        row = run(item["prompt"], sess, max_turns=item.get("max_turns", 20))
        row["id"] = pid
        row["watch"] = item.get("watch", "")
        rows.append(row)
        dest.write_text(json.dumps(rows, indent=2))
        flag = " ⚠RATE" if row["rate_limited"] else ""
        print(f"    {row['seconds']}s in={row['tokens_in']} tools={row['tools']}{flag}", flush=True)
        print("    " + (row["reply"][:500] or "(EMPTY)").replace("\n", "\n    "), flush=True)
        if i < len(todo):
            time.sleep(PACE_SECONDS)
    print(f"\nwrote {dest}", flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
