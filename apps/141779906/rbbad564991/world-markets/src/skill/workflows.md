# Workflows

## Account and portfolio

1. Call `get_world_account` before answering any question about balances, buying-power inputs, lending or borrowing, perpetual exposure, account value, or liquidation eligibility.
2. Use the account ID, owner, actor, and authorization returned by the tool in the answer so the user can tell which World account was inspected and whether the agent grant is active.
3. Name the relevant assets and positions. If the tool does not return a requested field, say that it is unavailable rather than estimating it.
4. Use `get_world_agent_permission` when the user asks whether access is active or revoked.

## Asset and market discovery

1. Call `list_world_assets` before translating an unfamiliar symbol into a token ID.
2. Call `get_world_market` to verify that a spot, perpetual, or lending market exists and to obtain its live order-book address and mark price.
3. Never invent a token ID, market address, mark price, or product pairing.

## Trade preview

1. Resolve the World account and market before discussing the concrete effect of a trade.
2. Call `preview_world_trade` or `check_world_mandate` with the user's exact product, side, symbols, and quantity.
3. Present the returned policy status, rule, and detail exactly. Never invent or soften a denial reason.
4. Present the account, side, quantity, current position, mark price, estimated notional, pre-execution risk value, and liquidation state from the preview.
5. If post-trade risk cannot be proven, report `post_trade_risk_unavailable` as a fail-closed denial. Never reinterpret it as permission.
6. Even a future `allow` verdict remains non-executable in this release. Describe it as mandate permission for the intent, not an order or signing request.

## Resting orders

Call `get_world_open_orders` for a specific spot or perpetual market before discussing whether an order is still resting. An absent order is not proof of a fill; it may have filled, been cancelled, or expired outside this app's observation.

For unsupported intents such as placing, cancelling, signing, or submitting an order, explain the current boundary directly and continue helping with the live mandate check, order read, or preview that is available.
