#!/usr/bin/env python3
"""Run the local UniFi trading suite through aomi-run. Development only."""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
import time
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SUITE = Path(__file__).resolve().parent
HASH_RE = re.compile(r"0x[a-fA-F0-9]{64}")
ERROR_RE = re.compile(r"Error:\s*`?(?:\[world-markets\])?([^`\n]+)`?", re.I)
TICKER_RE = re.compile(r"\b(WETH|USDT|WBTC|SOL|ETH|BTC)\b")


def plugin_path() -> Path:
    dylib = ROOT / "target/debug/libworld_markets.dylib"
    so = ROOT / "target/debug/libworld_markets.so"
    if dylib.exists():
        return dylib
    if so.exists():
        return so
    raise SystemExit("plugin not built; run cargo build")


def sidecar_health(port: str) -> bool:
    try:
        subprocess.run(
            ["curl", "-sf", f"http://127.0.0.1:{port}/health"],
            check=True,
            capture_output=True,
        )
        return True
    except subprocess.CalledProcessError:
        return False


def passed(trade: dict, stdout: str, hashes: list[str], returncode: int) -> tuple[bool, str | None]:
    hard = ERROR_RE.search(stdout)
    if trade.get("expect") == "markets":
        tickers = {m.group(1) for m in TICKER_RE.finditer(stdout)}
        ok = returncode == 0 and "Error:" not in stdout and len(tickers) >= 3
        return ok, None if ok else f"need ≥3 tickers, saw {sorted(tickers)}"
    needed = int(trade.get("min_hashes", 1))
    if returncode != 0:
        return False, hard.group(1).strip() if hard else f"exit {returncode}"
    if hard and len(hashes) < needed:
        return False, hard.group(1).strip()
    if len(hashes) < needed:
        return False, f"need {needed} tx hash(es), saw {len(hashes)}"
    return True, None


def run_trade(plugin: Path, trade: dict, session: str, env: dict[str, str]) -> dict:
    stdout_path = SUITE / "logs" / f"{trade['id']:02d}-{trade['name']}.stdout"
    stderr_path = SUITE / "logs" / f"{trade['id']:02d}-{trade['name']}.stderr"
    cmd = [
        "aomi-run",
        str(plugin),
        "--env-file",
        str(ROOT / ".env"),
        "--provider",
        "openrouter",
        "--session-id",
        session,
        "--max-turns",
        "12",
        "--prompt",
        trade["prompt"],
    ]
    started = time.time()
    proc = subprocess.run(cmd, cwd=ROOT, capture_output=True, text=True, env=env)
    stdout_path.write_text(proc.stdout)
    stderr_path.write_text(proc.stderr)
    hashes = list(dict.fromkeys(HASH_RE.findall(proc.stdout)))
    ok, error = passed(trade, proc.stdout, hashes, proc.returncode)
    return {
        "id": trade["id"],
        "name": trade["name"],
        "coverage": trade["coverage"],
        "ok": ok,
        "exit_code": proc.returncode,
        "seconds": round(time.time() - started, 1),
        "transaction_hashes": hashes,
        "error": error,
        "reply": proc.stdout.strip(),
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--from-id", type=int, default=1)
    parser.add_argument("--to-id", type=int, default=0)
    args = parser.parse_args()
    (SUITE / "logs").mkdir(exist_ok=True)
    cases = json.loads((SUITE / "cases.json").read_text())
    trades = [
        t
        for t in cases["trades"]
        if t["id"] >= args.from_id and (args.to_id == 0 or t["id"] <= args.to_id)
    ]
    plugin = plugin_path()
    port = os.environ.get("WORLD_EXECUTION_PORT", "8787")
    if not sidecar_health(port):
        raise SystemExit(
            f"execution sidecar is not healthy on 127.0.0.1:{port}; start it first"
        )
    env = os.environ.copy()
    env["WORLD_MANDATE_PATH"] = str(SUITE / "mandate.json")
    env.pop("WORLD_MANDATE_JSON", None)
    session = f"wm-suite-{datetime.now(timezone.utc).strftime('%Y%m%dT%H%M%SZ')}"
    results = []
    print(f"plugin={plugin}")
    print(f"session={session}")
    print(f"mandate={env['WORLD_MANDATE_PATH']}")
    print(f"trades={len(trades)}")
    for i, trade in enumerate(trades, 1):
        print(
            f"\n[{i}/{len(trades)}] {trade['id']:02d} {trade['name']} — {trade['coverage']}",
            flush=True,
        )
        row = run_trade(plugin, trade, session, env)
        results.append(row)
        if row["ok"]:
            print(f"  PASS  hashes={row['transaction_hashes']} ({row['seconds']}s)", flush=True)
        else:
            print(
                f"  FAIL  error={row['error']!r} hashes={row['transaction_hashes']} ({row['seconds']}s)",
                flush=True,
            )
            preview = row["reply"][-800:] if row["reply"] else ""
            if preview:
                print(preview, flush=True)

    out = {
        "ran_at": datetime.now(timezone.utc).isoformat(),
        "session": session,
        "passed": sum(1 for row in results if row["ok"]),
        "failed": sum(1 for row in results if not row["ok"]),
        "results": results,
    }
    (SUITE / "results.json").write_text(json.dumps(out, indent=2) + "\n")
    total = len(results)
    lines = [
        "# Trading suite results",
        "",
        f"Ran at `{out['ran_at']}` session `{session}`.",
        f"**{out['passed']}/{total} passed**, {out['failed']} failed.",
        "",
        "| # | name | result | hashes |",
        "|---|---|---|---|",
    ]
    for row in results:
        mark = "PASS" if row["ok"] else "FAIL"
        hashes = "<br>".join(f"`{h}`" for h in row["transaction_hashes"]) or "—"
        extra = f" {row['error']}" if row["error"] else ""
        lines.append(f"| {row['id']} | `{row['name']}` | {mark}{extra} | {hashes} |")
    (SUITE / "results.md").write_text("\n".join(lines) + "\n")
    print(f"\n{out['passed']}/{total} passed, {out['failed']} failed")
    print(f"wrote {SUITE / 'results.md'}")
    return 0 if out["failed"] == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
