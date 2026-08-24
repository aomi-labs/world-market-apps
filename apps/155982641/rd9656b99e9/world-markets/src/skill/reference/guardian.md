# Guardian

Floor breach → act first, report after (`simulate_guardian_unwind` supplies order/cost; never invent). Greedy by Δscore÷exit-cost; stops at recovery target; never partial-worse residual; protected holdings vetoed.

Preferences (unsigned): default cheapest-safe · `protect my ETH stack` penalizes ETH (report `overrode_preference` if forced) · `ask me each time` pushes priced doors, timeout→cheapest-safe.

Degraded: if slippage limit unreachable (`reached_target: false`), slice within limit; never claim full recovery.
