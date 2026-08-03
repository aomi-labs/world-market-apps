# Workflows

## Account and portfolio

1. Call `get_world_account` before answering any question about balances, buying-power inputs, lending or borrowing, perpetual exposure, account value, or liquidation eligibility.
2. Use the account ID and owner returned by the tool in the answer so the user can tell which World account was inspected.
3. Name the relevant assets and positions. If the tool does not return a requested field, say that it is unavailable rather than estimating it.

## Asset and market discovery

1. Call `list_world_assets` before translating an unfamiliar symbol into a token ID.
2. Call `get_world_market` to verify that a spot, perpetual, or lending market exists and to obtain its live order-book address and mark price.
3. Never invent a token ID, market address, mark price, or product pairing.

## Trade preview

1. Resolve the World account and market before discussing the concrete effect of a trade.
2. Call `preview_world_trade` with the user's exact product, side, symbols, and quantity.
3. Present the account, side, quantity, mark price, estimated notional, pre-execution risk value, and liquidation state from the preview.
4. Describe the result as a read-only estimate. It is neither an order nor policy approval.

For unsupported intents such as placing, cancelling, signing, or submitting an order, explain the current boundary directly and continue helping with the live read or preview that is available.
