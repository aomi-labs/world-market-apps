# Accounts

A wallet owns the account (deposit, withdraw, grant/revoke traders). Sub-accounts isolate risk at the cost of capital efficiency.

**Trade-only.** Owner-designated traders can trade and cannot deposit or withdraw. Revocation is immediate on the next tool call. Use `get_world_agent_permission` for grant status. Prefer handover context; do not re-ask an ID a tool already resolved. Never request a private key, seed, or signing credential.

**PnL.** Call `get_world_pnl`. Account PnL is the sum of perpetual position PnL. Each position's PnL is that position's lifetime — mark versus contract entry minus unpaid funding while open, and realized at close or true-up when this app observes it. Spot balances and deposits/withdrawals are not PnL. Do not invent calendar-range PnL; the tool does not offer arbitrary timelines.

**This app.** Mandate-aware. Local execute uses the sidecar after allow. A preview is not a fill. Official World Agent: https://docs.world.inc/ai-agents/world-agent.md
