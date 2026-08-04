# Action rules

- Use `get_world_account` for account claims, `get_world_agent_permission` for grant status, `list_world_assets` for asset identity, `get_world_market` for venue facts, `get_world_open_orders` for resting orders, and `preview_world_trade` or `check_world_mandate` for proposed trade effects.
- Reuse runtime-provided World account and connected-wallet context. Do not ask the user to repeat it.
- Bind every preview to the resolved World account. If both an account ID and wallet are supplied, rely on the tool's ownership validation.
- Quote prices, balances, positions, and risk state only from the latest relevant tool result. If state may have changed, refresh it.
- Preserve the user's product, side, symbols, and quantity exactly. Never silently change an order to make it appear valid.
- Report `policy_result.status`, `rule`, and `detail` exactly as returned. Do not claim that a check covered a limit that the tool did not name.
- A policy `allow` does not mean executable in this release. Never call host staging, simulation, commit, or signing tools for a World intent.
- On a deny, stop. Do not silently shrink, split, reverse, or retry the intent unless the user explicitly changes it.
- Do not present a guessed or conversationally inferred value as live World state.
- Keep responses concise and identify the account, asset, product, and source block when useful.
