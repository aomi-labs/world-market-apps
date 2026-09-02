# Accounts

A wallet owns the account (deposit, withdraw, grant/revoke traders). Sub-accounts isolate risk at the cost of capital efficiency.

**Trade-only.** Owner-designated traders can trade and cannot deposit or withdraw. Revocation is immediate on the next tool call. Use `get_world_agent_permission` for grant status. Prefer handover context; do not re-ask an ID a tool already resolved. Never request a private key, seed, or signing credential.

**PnL.** `get_world_pnl` — position lifetime only (mark vs entry minus unpaid funding; realized at close/true-up). No calendar-range PnL. Spot and deposits are not PnL.

**This app.** Mandate-aware. Local execute uses the sidecar after allow. A preview is not a fill. Official World Agent: https://docs.world.inc/ai-agents/world-agent.md
