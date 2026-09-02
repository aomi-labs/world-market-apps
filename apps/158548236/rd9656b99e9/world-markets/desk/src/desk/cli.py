from __future__ import annotations

import argparse
import sys
from pathlib import Path

from dotenv import load_dotenv

from desk.config import load_config
from desk.persist import Store, replay_text


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(prog="desk", description="The Desk")
    sub = parser.add_subparsers(dest="cmd", required=True)
    p_serve = sub.add_parser("serve", help="local mock room + API")
    p_serve.add_argument("--config", type=Path, default=None)
    p_tape = sub.add_parser("tape", help="tape utilities")
    tape_sub = p_tape.add_subparsers(dest="tape_cmd", required=True)
    p_replay = tape_sub.add_parser("replay")
    p_replay.add_argument("session_id")
    p_replay.add_argument("--db", type=Path, default=None)
    args = parser.parse_args(argv)

    if args.cmd == "serve":
        from desk.earcons import ensure_earcons
        from desk.server import serve

        repo_root = Path(__file__).resolve().parents[3]
        desk_root = Path(__file__).resolve().parents[2]
        load_dotenv(repo_root / ".env")
        load_dotenv(desk_root / ".env")
        cfg = load_config(args.config)
        ensure_earcons(Path(__file__).resolve().parents[2] / "assets" / "earcons")
        serve(cfg)
        return 0
    if args.cmd == "tape" and args.tape_cmd == "replay":
        cfg = load_config()
        db = args.db or Path(cfg.data_dir) / "desk.sqlite"
        store = Store(f"sqlite:///{db}")
        sys.stdout.write(replay_text(store, args.session_id))
        return 0
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
