# ATLAS

Portfolio-level risk: one available-margin figure; liquidation when margin cannot support the risk.

**Margin inputs:** spot notional; 98% lender loan notional; unrealized perp PnL; hedged underlying netting; minus 10-day borrow interest and shock on unhedged exposure.

**Netting:** equal spot + short perp on same underlying have no directional ETH risk under ATLAS. Basis risk remains.

**Borrowing:** liability + Borrow position. Interest fixed 10 days. Lent capital still margins (~98% available).

**Liquidation:** available margin at 0. Negative RAPV = eligibility. Unproven post-trade RAPV → fail closed.

Math: https://docs.world.inc/details/atlas-math-risk-based-valuation.md
