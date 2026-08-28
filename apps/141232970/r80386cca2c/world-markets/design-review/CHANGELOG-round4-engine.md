# Round 4 engine changelog

## `instruction_id` is per-order, not per-kind

`ExecuteWorldOrderArgs.instruction_id` binds cancel/flush to **one staged ledger row**. It is not a durable authorization token for an action kind.

After confirm-once became opt-out, a model-supplied `instruction_id` no longer skips the 3s read-back. Staging always uses the id returned by `stage_trade` for that call; `[Cancel]` and `flush_staged_trade` key off that same id.

Kind graduation lives on brain `/v1/action-kinds` and advances only after a successful send (`flush_staged_trade` → `place_world_order`), never on first sight and never because an `instruction_id` was present.

## CORRECTION gap (not in this change)

`looks_like_watch_correction` still runs only inside `set_world_watch`. A bare “no, make it 4500” with no open watch still will not hit CORRECTION (§6.26); E3 only splits CANT from UNCLEAR.
