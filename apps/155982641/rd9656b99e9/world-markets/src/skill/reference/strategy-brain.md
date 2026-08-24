# Strategy brain

Rank internally; one recommendation. Tools prove numbers; this file picks path and timing.

## Doctrine

D1 Operate, don't menu · D2 Continuous yield > episodic · D3 Counterparties roll · D4 tools prove numbers · D5 mandate > doctrine > preferences · D6 size for the floor.

## Ranking (internal order)

HEDGE (0) → DEPLOY (1) → LEND/REBAL (2) → BASIS (3). Risk before yield, always; basis only on a positive spread that clears entry+exit cost.

## Rate & timing

Annualize funding ×`1095` (tool) before comparing to borrow. Native yield is a spot-token property, never netted against lend; missing → "unknown". Roll at maturity if live lend > expiring net of cost; honor `extensible`. Negative carry: day N of the receipt's trigger closes.

## Loop (material recommendations)

Refresh account, markets, carry → rank internally → one conclusion + next action. Compare only on request.

## Anti-patterns

false binary · deferral · product buffet · idle cash when PB-DEPLOY applies · thin-spread chase.

## Playbooks

| id | pri | action | notes |
|----|-----|--------|-------|
| PB-DEPLOY | 1 | auto-earn idle quote (still ~98% margin) | not parked; honor size unit + weights |
| PB-LEND | 2 | fixed lend if rate beats PB-DEPLOY | 10d, auto-extend unless locked; roll at maturity |
| PB-BASIS | 3 | borrow, spot long + perp short | funding > borrow; §6.9 exits after N negative days; spread-widening can liquidate |
| PB-HEDGE | 0 | `simulate_guardian_unwind` | floor / negative RAPV; risk ≥ 8 → no new exposure |
| PB-REBAL | 2 | rebalance to targets inside caps | max notional + leverage |

## Regime & triggers

Basis-rich (funding > lend) → PB-BASIS + PB-DEPLOY. Converged → PB-DEPLOY / PB-LEND. Negative carry → exit basis §6.9. Risk ≥ 8 or negative RAPV → PB-HEDGE. Spread flip each 8h tick; loan maturity → roll; idle quote → PB-DEPLOY; floor breach → unwind now.