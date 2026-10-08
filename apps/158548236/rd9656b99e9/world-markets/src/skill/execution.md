# World Markets — execution procedure

Every World transaction leaves this app through the host's staging pipeline, guarded to the exchange contract. The steps below are the only way an order, cancel, or loan action reaches the chain. Run them in order, in silence, and stop at the first failure.

## Constants

- `EXCHANGE` = `0xf6b54e033bb45a583aa642924bcef78b804588ae` (chain 2092151908). Every staged call has `to = EXCHANGE`.
- Every trading function takes `(address book, uint256 word)`: the book contract from `world_resolve_book` and the packed order word from `world_pack_order`.
- Allowed functions: `newSpotBuyOrder` · `newSpotSellOrder` · `newPerpBuyOrder` · `newPerpSellOrder` · `newLendOrder` · `newBorrowOrder` · `cancelSpotBuyOrder` · `cancelSpotSellOrder` · `cancelPerpBuyOrder` · `cancelPerpSellOrder` · `cancelLendOrder` · `cancelBorrowOrder` · `renewLoan(uint64,uint64,uint64)` · `payInterestAndFees(uint64,uint64,bool)`. Nothing else, ever.

## New order (spot or perp)

1. **Verdict.** `preview_world_trade` with product, side, symbols, the user's whole sentence as `text` (plus `price` / `order_type` / `slippage` when named). Read `preview.verdict.status`.
   - `deny` → stop. Nothing is resolved, packed, or staged. Report the block per the reporting skill.
   - `allow` → continue with this preview's fields only. Never re-type a number.
2. **Book.** `world_resolve_book { product, base_token_id: preview.base.token_id, quote_token_id: preview.quote.token_id }` → `book`. `error: no_book` → stop; there is no market.
3. **Word.** `world_pack_order { account_id: preview.account_id, quantity_raw: preview.quantity_raw, limit_price: preview.limit_price, order_type }` → `word`.
   - `order_type`: `limit` when `preview.order_type` is `limit`; `fill_partial_kill_rest` when it is `market` (immediate fill at up to the slippage-moved limit, remainder cancelled). Use `fill_all_or_revert` only when the user said all-or-nothing.
4. **Stage.** `evm_stage_tx` in Encode mode, once:
   - `to`: `EXCHANGE`
   - `data`: `{ "signature": "<function>(address,uint256)", "args": [book, word] }` with `<function>` chosen by product and side: spot buy → `newSpotBuyOrder`, spot sell → `newSpotSellOrder`, perp buy → `newPerpBuyOrder`, perp sell → `newPerpSellOrder`.
   - `value`: `0`. Example: `signature: "newSpotSellOrder(address,uint256)"`, `args: ["0x…book", "0x…word"]`.
5. **Simulate.** `simulate_batch` once with the complete staged list. A revert or a guard block → stop; nothing is committed. Report the reason verbatim.
6. **Commit.** `evm_commit_txs` once with that same complete list. Never split the list, never retry a commit, never fall back to EOA execution. The receipt hash from the host is the only fill evidence.

One user instruction = one preview, one stage, one simulate, one commit. A second order starts again at step 1.

## Cancel a resting order

1. `get_world_open_orders` for the product and pair → the order's `order_id`, `side`, `quantity_raw`, `price_raw`, and `order_book`. No matching order → stop and say so.
2. `world_resolve_book` for the same pair (or reuse `order_book`).
3. `world_pack_order` with the resting order's `account_id`, `quantity_raw`, `price` (decimal, as shown), `order_type: limit`, and `insertion_hint: order_id`.
4. `evm_stage_tx` Encode mode, `to = EXCHANGE`, signature by product and side: `cancelSpotBuyOrder` / `cancelSpotSellOrder` / `cancelPerpBuyOrder` / `cancelPerpSellOrder` `(address,uint256)`, `args: [book, word]`.
5. `simulate_batch` once → `evm_commit_txs` once.

A cancel never trades. Lend/borrow order cancels use `cancelLendOrder` / `cancelBorrowOrder` with the lend book from `world_resolve_book { product: "lend", base_token_id }`.

## Loans

Loan actions need a bound, unexpired mandate and a live account above its floor. `compute_resize` fails with `missing_mandate` / `expired_mandate` when no mandate is bound — stop there. `get_world_account` with `eligible_for_liquidation: true`, or `risk_adjusted_portfolio_value` below the floor, also stops the flow. Then, per loan from `get_world_loans`:

- Renew: `evm_stage_tx` Encode, `to = EXCHANGE`, `signature: "renewLoan(uint64,uint64,uint64)"`, `args: [account_id, account_id, position_id]` — the venue signature is `renewLoan(user, userToPay, almostDueLendingId)`; the borrower pays for itself, so both account arguments are the bound account and the third is the loan's `position_id` from `get_world_loans`.
- Pay interest / fees: `signature: "payInterestAndFees(uint64,uint64,bool)"`, `args: [position_id, reduce_quantity_raw, extend_period]` — the venue signature is `payInterestAndFees(positionId, reduceQuantity, extendPeriod)`; `reduce_quantity_raw` is `0` unless the user asked to reduce the loan, and `extend_period` is `true` only when the user asked to extend.

Stage every loan call for the instruction, then one `simulate_batch`, then one `evm_commit_txs` for the complete ordered list.

## Never

- Never call `batchCommands`, `liquidate`, `bankruptcy`, `lifoLenderSwap`, `requestPerpTrueUp`, `markLendPositionAsNonReturnable`, or any deposit, withdrawal, transfer, approval to another spender, or bridge. The guard blocks them; a block is a report, not a retry.
- Never stage to any address other than `EXCHANGE`; never stage on another chain.
- Never construct a word by hand, never edit a word, never change `quantity_raw` or `limit_price` between preview and pack.
- Never stage after a `deny`, after `no_book`, or after a failed simulation.
- Never split an atomic list, never commit twice, never sign or broadcast outside `evm_commit_txs`.
- Never ask for a key, seed, or credential; the host's mandate-aware AA gate signs from the trading account.

## After commit

Report from the host receipt and a fresh `get_world_account`: what happened (`~$[#] of [asset] [product]`, filled at `[#]`), why (the user's ask), account effect, policy (`within limits`), next. Missing hash → say the commit did not return one; never claim a fill.
