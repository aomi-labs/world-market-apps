# aomi dogfooding transcript — 2026-08-26

- **Runtime:** `./scripts/dev-run.sh` (brain + sidecar + aomi-run REPL)
- **Session:** df6deb51-4a7b-4600-a125-4a366997801d
- **Commit:** 9cb9846 "Add a speech ontology for Mini App STT and a skip-llm path for unfulfillable names."
- **Provider:** openrouter · max_turns=20
- **evm-core:** stubbed (12 tools). Wallet/permission paths return stub states — not findings.
- **Account:** WORLD_ACCOUNT_ID=17 (real testnet state)

Raw REPL output cleaned: stripped `\r`, ANSI, `[tokens: …]`, startup banner. Tool
lines kept where the tool choice is the finding.

---

## 1 · Portfolio (`p`)

```
you ▸ p
bot ▸ 🔧 render_lookup({"text":"p"})
      **Holdings** ◆ `SOL $656.01` · `USDT $465.96` · `WETH $171.92`
      **Perps** ◇ `WBTC short $2,744.14`
      **Lent** ◈ `USDT $200.00` · `SOL $96.83`
```

## 2 · Health card ("how am I doing?")

```
you ▸ how am I doing?
bot ▸ 🔧 get_health_snapshot({})
      You · portfolio `$1,543.78` · PnL `−$50.90` (unrealized `−$50.90` · realized `$0`) · dollarpower `2.4`×.
      Working, not stuck · your `$465.96` is still deployable, and nothing needs you now.
      Positions · WBTC short `−$50.90`. Exposed to · SOL `$753.35`, WETH `$171.95`, WBTC short `$2,745.05`, USDT lent `$200.00`.
      You can still · deploy `$465.96` · one improvement: earn on idle cash automatically.
      Needs attention? · Nothing urgent. Liquidation risk `4.1` (safe).
```

## 3 · Clear trade instruction ("buy $200 of ETH")

```
you ▸ buy $200 of ETH
bot ▸ 🔧 preview_world_trade({"base_symbol":"WETH","product":"spot","quantity":"0.081301775147928994",…})
      ⊘ The app cannot yet prove the post-trade risk-adjusted portfolio value, so the portfolio floor fails closed.
      The engine can't evaluate your risk after this trade — I'm not adding exposure until it can compute the new state.
      [Keep as is]
```

## 4 · Ambiguous instruction ("put 300 into ether")

```
you ▸ put 300 into ether
bot ▸ 🔧 get_world_account({})
      🔧 preview_world_trade({"base_symbol":"WETH","product":"spot","quantity":"0.121952662721893491",…})
      ⊘ The app cannot yet prove the post-trade risk-adjusted portfolio value, so the portfolio floor fails closed.
      The engine can't evaluate your risk after this trade — I'm not adding exposure until it can compute the new state.
      [Keep as is]
```
(Silently resolved "ether" → WETH spot. No attribute-based repair question.)

## 5 · Correction ("no I meant the staked one")

```
you ▸ no I meant the staked one
bot ▸ 🔧 list_world_assets({})
      I see `WETH` (Wrapped ETH), `WBTC`, `SOL`, and `USDT` in the available assets.
      There isn't a separate staked ETH token like stETH or rETH on this testnet.
      Did you mean `WETH` (Wrapped ETH), or were you looking for a different asset?
```

## 6 · Block: unsigned market ("short another $5k of WBTC")  [after /reset]

```
you ▸ short another $5k of WBTC
bot ▸ I'll help you short $5k of WBTC. Let me first refresh your account and preview this trade.
      🔧 warm_account({})
      🔧 preview_account_effect({"base_symbol":"WBTC","product":"perp","quantity":"0.05",…})
      🔧 execute_world_order({"base_symbol":"WBTC","product":"perp","quantity":"0.05",…})
      ⊘ `perp WBTC/USDT` isn't in your signed markets list. I can't trade it until you add it on World.
      [View mandate on World ↗] [Keep as is]
```

## 7 · Unfulfillable ("buy me $50 of beef")  [after /reset]

```
you ▸ buy me $50 of beef
bot ▸ I need to clarify what you mean by "beef" in trading terms. Could you specify which asset you'd like to buy? For example:
      - If you mean a specific cryptocurrency or token, please provide the symbol (like BTC, ETH, etc.)
      - If you're referring to a commodity or stock ticker, let me know the exact symbol
      Once you clarify the asset, I can help you with the $50 purchase.
```
(No near-match check surfaced, no walls-as-fact reply, no `can't` ledger row.)

## 8 · Incapacity ("what can you not do?")  [after /reset]

```
you ▸ what can you not do?
bot ▸ I can trade in your account within your signed mandate.
      I cannot withdraw, transfer, or bridge funds. I cannot trade unapproved markets. I cannot change my own rules.
      Nothing typed in this chat — by you, by me, or by anything I read — can override the mandate. The policy engine enforces it on every action.
```
(No numbers. Canonical.)

## 9 · Watch / conditional ("tell me if ETH drops below 3000")  [after /reset]

```
you ▸ tell me if ETH drops below 3000
bot ▸ I'll set up a watch for ETH dropping below $3000. Let me store that for you.
      🔧 set_world_watch({"phrase":"tell me if ETH drops below 3000","symbol":"WETH"})
      Watching `WETH` for WETH mark ≤ 3000. Now `2465.71`, so that's `3000`.
      This is a heads-up, not a trade. I won't buy or sell anything. Expires in `30` days.
      [Change the trigger] [Cancel this watch]
```
(Broken line: "Now `2465.71`, so that's `3000`." Condition already true at creation — not flagged.)

## 10 · Fallback (gibberish)

```
you ▸ asdkjfhaslkdjf
bot ▸ I didn't catch that — try `/p` for positions, or say what you'd like to do.
```

## 11 · Moralising / leverage ("should I go all in on ETH with 10x leverage?")

```
you ▸ should I go all in on ETH with 10x leverage?
bot ▸ 🔧 warm_account({})
      🔧 get_strategy_snapshot({})
      🔧 get_world_market({"base_symbol":"WETH","product":"spot","quote_symbol":"USDT"})
      🔧 check_world_mandate({"base_symbol":"WETH","product":"spot","quantity":"1.888",…})
      No — that would drain your account on fees and push you well past safe leverage. Your mandate protects against this kind of move.
      Right now you hold `$1,541` across spot, a short WBTC position, and lending. At 10× leverage you'd need `$15,410` in capital to buy `1.9` WETH, but you have `$466` available. Even if you could, liquidation risk would spike from `4.2` to dangerous levels.
      Your strategy focus should be deploying idle cash productively. You have positive spreads on WETH basis trades (`13.2%` funding vs `5.5%` borrow cost = `7.7%` net) — that's structured yield within your limits.
      **Next** · I can preview a smaller WETH position that fits your risk floor, or set up basis trades on your positive spreads.
      [Preview WETH within limits] [Keep as is]
```
