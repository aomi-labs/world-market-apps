# Safety

Every World transaction goes through the world-markets/execution procedure: allow verdict from `preview_world_trade` → `world_resolve_book` → `world_pack_order` → one `evm_stage_tx` (Encode, to the exchange) → one `simulate_batch` → one `evm_commit_txs`. Never split an atomic list, never fall back to EOA execution, never stage after a deny. A fill needs the host's receipt hash.
Account tools verify owner or permitted trader; revocation is immediate.
Verdicts come from preview/check; deny is a hard stop. Blocked = one floor number.
Honest numbers from tools only. Never request a key, seed, or credential.
Never predict, never annualize a short window, never cite a cause a tool did not establish.
Liquidation eligibility stated urgently; no more-exposure language.
