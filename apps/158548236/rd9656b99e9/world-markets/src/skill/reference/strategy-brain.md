# Strategy brain

Rank internally; one recommendation. Tools prove numbers; this file picks path and timing.

## Doctrine

D1 Operate, don't menu · D2 Continuous yield > episodic · D3 Counterparties roll · D4 tools prove numbers · D5 mandate > doctrine > preferences · D6 size for the floor.

## Ranking (internal order)

HEDGE (0) → DEPLOY (1) → LEND/REBAL (2) → BASIS (3). Risk before yield, always; basis only on a positive spread that clears entry+exit cost.

## Rate & timing

Annualize funding vs borrow from tools. Native yield is a spot-token property. Roll at maturity; honor `extensible`.

## Loop (material recommendations)

`get_strategy_snapshot` → rank → one conclusion. Compare only on request.

## Anti-patterns

false binary · deferral · product buffet · idle cash · thin-spread.

## Playbooks

| id | pri | action |
|----|-----|--------|
| PB-DEPLOY | 1 | auto-earn idle quote (~98% margin) |
| PB-LEND | 2 | fixed lend if it beats PB-DEPLOY |
| PB-BASIS | 3 | borrow, spot long + perp short |
| PB-HEDGE | 0 | `simulate_guardian_unwind` |
| PB-REBAL | 2 | rebalance to targets inside caps |

## Regime & triggers

Funding > lend → PB-BASIS + PB-DEPLOY. Else PB-DEPLOY / PB-LEND. Negative carry → §6.9. Risk ≥ 8 or RAPV<0 → PB-HEDGE.
