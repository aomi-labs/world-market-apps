//! Deterministic reporting service (the "honest-numbers layer").
//!
//! Every number that appears in a rendered Telegram message must originate here
//! or from a live contract read — never from the language model. This module is
//! the source of the *derived* figures the message templates interpolate: yield
//! and risk deltas, resize solutions, exit costs, slice savings, dollarpower, and
//! the guardian cheapest-safe unwind plan.
//!
//! Spec references are to `TELEGRAM-MESSAGING-UX-SPEC.md`:
//! - §4.1 honest-numbers layer: the model writes only the sentences *between*
//!   these numbers; net-of-costs by default; every counterfactual names its
//!   baseline; null results are results ("≈ $0 difference").
//! - §11 integration contracts: these types are the placeholders the renderer
//!   must tolerate until the real reporting service exists. The engine exposes a
//!   RAPV floor, not a 0–10 score, so risk is carried in engine units as strings.
//!
//! All money/rate/quantity fields are `String` (decimal, exact) so nothing is
//! silently reformatted and no `f64` rounding leaks into a receipt. Arithmetic is
//! done here in Rust with `rust_decimal`, never by the model.

use rust_decimal::Decimal;
use serde::Serialize;

/// A single figure paired with the metadata the copy layer needs to render it
/// honestly: whether it is an estimate (vs an exact contract value) and the
/// baseline any counterfactual is measured against (§4.1).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct Figure {
    /// The exact decimal value, as a string. Never a float.
    pub(crate) value: String,
    /// Unit label, e.g. "USDT", "%", "×", "ETH". Empty for a bare count.
    pub(crate) unit: String,
    /// True when the value is a simulated/estimated figure rather than an exact
    /// contract read. Drives the "estimate, not a measured alternative" wording.
    pub(crate) is_estimate: bool,
}

impl Figure {
    pub(crate) fn estimate(value: impl Into<String>, unit: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            unit: unit.into(),
            is_estimate: true,
        }
    }

    pub(crate) fn exact(value: Decimal, unit: impl Into<String>) -> Self {
        Self::decimal(value, unit, false)
    }

    pub(crate) fn decimal(value: Decimal, unit: impl Into<String>, is_estimate: bool) -> Self {
        Self {
            value: value.normalize().to_string(),
            unit: unit.into(),
            is_estimate,
        }
    }
}

/// A before→after transition of a single figure. The renderer shows
/// "`before` → `after`"; both come from the service so the arrow never spans a
/// model-invented number.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct Transition {
    pub(crate) before: String,
    pub(crate) after: String,
    pub(crate) unit: String,
    /// True when nothing changed; the message layer must suppress this line (F4a).
    pub(crate) unchanged: bool,
    /// Direction word from the reporting layer — never inferred from raw numbers in copy.
    /// `liquidation_risk` (0–10, higher = worse): `safer` / `less safe`.
    /// `rapv` (engine-internal, higher = safer): `safer` / `less safe`.
    /// Other fields: `rises` / `falls` / `unchanged`.
    pub(crate) direction: String,
}

impl Transition {
    pub(crate) fn new(
        before: Decimal,
        after: Decimal,
        unit: impl Into<String>,
        field: &str,
    ) -> Self {
        let unchanged = before == after;
        let direction = direction_word(before, after, field);
        Self {
            before: before.normalize().to_string(),
            after: after.normalize().to_string(),
            unit: unit.into(),
            unchanged,
            direction,
        }
    }
}

/// One polarity mapping per risk field. Do not put RAPV and the 0–10 score
/// under the same `risk` key — they invert.
pub(crate) fn direction_word(before: Decimal, after: Decimal, field: &str) -> String {
    if before == after {
        return "unchanged".to_string();
    }
    let rises = after > before;
    match field {
        // 0–10 liquidation score: higher = worse. A fall is safer.
        "liquidation_risk" => {
            if rises {
                "less safe".to_string()
            } else {
                "safer".to_string()
            }
        }
        // RAPV: higher = safer. Engine-internal; never the user-facing Risk line.
        "rapv" => {
            if rises {
                "safer".to_string()
            } else {
                "less safe".to_string()
            }
        }
        _ => {
            if rises {
                "rises".to_string()
            } else {
                "falls".to_string()
            }
        }
    }
}

/// §6.3 / §6.5 — the account-change bundle behind a preview or receipt.
/// User-facing Risk is the 0–10 liquidation score, never RAPV.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct AccountEffect {
    pub(crate) expected_net_yield: Option<Transition>,
    pub(crate) directional_exposure: Transition,
    pub(crate) exposure_symbol: String,
    pub(crate) available_to_deploy: Transition,
    /// 0–10 liquidation score. `None` when post-trade risk cannot be proven.
    pub(crate) liquidation_risk: Option<Transition>,
    pub(crate) estimated_cost: Option<Figure>,
    /// Concern-line direction for the score, verbatim. Absent when risk is omitted.
    pub(crate) direction: Option<String>,
    /// One portfolio-level clause. Never invented by the model.
    pub(crate) concern_clause: String,
    pub(crate) missing_mark_symbols: Vec<String>,
    pub(crate) post_trade_risk_unavailable: bool,
    pub(crate) baseline: String,
}

/// §6.6 / R3 — the resize solver output backing a block's "at $X · compliant".
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct ResizeSolution {
    /// The user's floor — the ONE number a block is allowed to cite (§4.3, R3).
    pub(crate) floor: Figure,
    /// Largest size that clears the floor; `None` when no positive size complies.
    pub(crate) largest_compliant_size: Option<Figure>,
    /// The engine `rule` code that gates this intent, verbatim.
    pub(crate) rule: String,
}

/// §6.17 / §05 — exit cost priced before entry is possible.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct ExitCost {
    pub(crate) price_impact: Figure,
    pub(crate) time_to_flat_p90: Figure,
    /// Net-of-everything result, e.g. "104.20 on 100 committed".
    pub(crate) net_result: Figure,
    pub(crate) baseline: String,
}

/// §6.16 — large-order slice plan and the money-saved story.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct SlicePlan {
    pub(crate) market_order_cost: Figure,
    pub(crate) sliced_cost: Figure,
    pub(crate) saved: Figure,
    pub(crate) slices: u32,
    pub(crate) window_minutes: u32,
    /// True when slicing makes no meaningful difference — copy must report the
    /// null result plainly ("$0 difference"), never a fabricated saving (§4.1).
    pub(crate) null_case: bool,
    pub(crate) baseline: String,
}

/// §6.15 — dollarpower (capital efficiency), always dollar-translated.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct Dollarpower {
    pub(crate) ratio: Figure,
    pub(crate) committed: Figure,
    pub(crate) effective: Figure,
}

/// A standing guardian preference set in chat (§6.8, R4). Never a signed policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GuardianPreference {
    /// Run the cheapest-safe algorithm unmodified (engine default).
    CheapestSafe,
    /// Penalize ETH-bearing candidates; chosen only if nothing else reaches the
    /// target, and reported honestly when overruled.
    ProtectEth,
}

/// One position/leg the guardian may close, with the deterministic inputs the
/// scoring function needs. All values come from the reporting service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UnwindCandidate {
    pub(crate) label: String,
    /// Risk-score points recovered if this leg fully closes (engine units).
    pub(crate) delta_score: Decimal,
    /// Slippage + fees (+ accrued interest for a loan leg) to fully close.
    pub(crate) exit_cost: Decimal,
    /// True when closing this leg leaves a *worse* residual (e.g. breaks a hedge
    /// into a naked directional position). The algorithm refuses such closes
    /// unless the residual is itself compliant.
    pub(crate) breaks_structure_into_worse_residual: bool,
    /// True when this leg reduces directional exposure (preferred).
    pub(crate) reduces_directional_exposure: bool,
    /// True when this holding is protected by policy — a veto, never a candidate.
    pub(crate) protected: bool,
    /// True when closing this leg touches ETH (penalized under `ProtectEth`).
    pub(crate) is_eth: bool,
}

/// One chosen step in the guardian plan, rendered in order (§6.8).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct UnwindStep {
    pub(crate) label: String,
    pub(crate) delta_score: String,
    pub(crate) exit_cost: String,
    /// True when this step was forced past an override preference (reported
    /// honestly per R4: "cheaper alternatives were exhausted").
    pub(crate) overrode_preference: bool,
}

/// §6.8 / R4 — the guardian cheapest-safe plan the message renders.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct UnwindPlan {
    pub(crate) steps: Vec<UnwindStep>,
    /// Total protection cost across chosen steps.
    pub(crate) cost_of_protection: String,
    /// Recovery target reached? False means the emergency-slippage-limited
    /// degraded state (§4 degraded): slice within the limit, hedge the residual,
    /// raise reporting frequency — never override the limit.
    pub(crate) reached_target: bool,
    /// Residual score after the plan (engine units).
    pub(crate) resulting_score: String,
    /// What a protection preference kept, if anything.
    pub(crate) kept: String,
}

/// §6.9 / R2 — the negative-carry regime state behind the pre-authorized plan.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct CarryState {
    pub(crate) position_id: String,
    pub(crate) entry_timestamp: String,
    pub(crate) negative_carry_window_days: u32,
    pub(crate) days_negative: u32,
    pub(crate) trigger_days: u32,
    pub(crate) avg_daily_carry: Figure,
    /// Latches true the first time `days_negative` reaches the window. Never resets.
    pub(crate) fired: bool,
    /// True on the day the plan fires and the position is closed after the fact.
    pub(crate) plan_executed: bool,
    /// Host runtime owns daily cadence; this plugin cannot schedule.
    pub(crate) cadence_owner: &'static str,
}

/// Recommended first deposit + the honest reason it travels with (owner, round 3).
/// A recommendation, not a gate. The message layer never hardcodes the amount.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct RecommendedDeposit {
    pub(crate) amount: Figure,
    pub(crate) rationale: String,
}

/// Canonical demo-book snapshot. Every showcase / drill figure is a field here.
/// The renderer interpolates; it never invents a number.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct DemoBook {
    pub(crate) committed: Figure,
    pub(crate) borrowed: Figure,
    pub(crate) spot: Figure,
    pub(crate) short: Figure,
    pub(crate) borrow_apr: Figure,
    pub(crate) daily_carry_net: Figure,
    pub(crate) worst_week_daily: Figure,
    pub(crate) negative_carry_close_days: Figure,
    pub(crate) dollarpower: Dollarpower,
    pub(crate) drill_move: Figure,
    pub(crate) drill_step1_label: String,
    pub(crate) drill_step1_freed: Figure,
    pub(crate) drill_step1_cost: Figure,
    pub(crate) drill_step2_label: String,
    pub(crate) drill_step2_repay: Figure,
    pub(crate) drill_step2_cost: Figure,
    pub(crate) drill_total_cost: Figure,
    pub(crate) drill_seconds: Figure,
    /// True when figures came from live market tools on the demo book.
    pub(crate) rates_live: bool,
}

/// The deterministic reporting service. The message layer reads numbers from
/// here; it never derives them itself.
pub(crate) trait Reporting {
    fn account_effect(&self, plan: &EffectPlan) -> AccountEffect;
    fn resize_solution(&self, input: &ResizeInput) -> ResizeSolution;
    fn exit_cost(&self, position_id: &str) -> ExitCost;
    fn slice_plan(&self, input: &SliceInput) -> SlicePlan;
    fn dollarpower(&self, portfolio_id: &str) -> Dollarpower;
    fn guardian_unwind(
        &self,
        candidates: &[UnwindCandidate],
        current_score: Decimal,
        recovery_target: Decimal,
        preference: GuardianPreference,
        emergency_slippage_reachable: bool,
    ) -> UnwindPlan;
    #[allow(dead_code)]
    fn carry_state(&self, position_id: &str) -> CarryState;
    /// Recommended first deposit for guest → funded conversion. Never a platform
    /// minimum. The renderer interpolates this; the model never types it.
    fn recommended_first_deposit(&self) -> RecommendedDeposit;
    /// Canonical illustrative book the guest showcase and fire drill run against.
    /// Production must populate this from live tools; the fixture is a stand-in.
    fn demo_book(&self) -> Result<DemoBook, String>;
}

/// Snapshot + intent evaluation the reporting layer formats. Every figure is
/// computed in Rust from live state (or a test fixture). The model never
/// supplies a before/after number.
#[derive(Debug, Clone)]
pub(crate) struct EffectPlan {
    pub(crate) exposure_symbol: String,
    pub(crate) exposure_before: Decimal,
    pub(crate) exposure_after: Decimal,
    pub(crate) available_before: Decimal,
    pub(crate) available_after: Decimal,
    pub(crate) quote: String,
    pub(crate) liquidation_risk_before: Option<Decimal>,
    pub(crate) liquidation_risk_after: Option<Decimal>,
    pub(crate) estimated_cost: Option<Decimal>,
    pub(crate) missing_mark_symbols: Vec<String>,
    pub(crate) post_trade_risk_unavailable: bool,
    pub(crate) concern_clause: String,
    pub(crate) baseline: String,
}

/// Format a derived plan. Yield is omitted until a live oracle exists.
pub(crate) fn derive_account_effect(plan: &EffectPlan) -> AccountEffect {
    let exposure = Transition::new(
        plan.exposure_before,
        plan.exposure_after,
        plan.quote.clone(),
        "exposure",
    );
    let available = Transition::new(
        plan.available_before,
        plan.available_after,
        plan.quote.clone(),
        "available",
    );
    let liquidation_risk = match (plan.liquidation_risk_before, plan.liquidation_risk_after) {
        (Some(before), Some(after)) => Some(Transition::new(before, after, "", "liquidation_risk")),
        _ => None,
    };
    let direction = liquidation_risk.as_ref().map(|t| t.direction.clone());
    AccountEffect {
        expected_net_yield: None,
        directional_exposure: exposure,
        exposure_symbol: plan.exposure_symbol.clone(),
        available_to_deploy: available,
        liquidation_risk,
        estimated_cost: plan
            .estimated_cost
            .map(|c| Figure::decimal(c, plan.quote.clone(), true)),
        direction,
        concern_clause: plan.concern_clause.clone(),
        missing_mark_symbols: plan.missing_mark_symbols.clone(),
        post_trade_risk_unavailable: plan.post_trade_risk_unavailable
            || plan.liquidation_risk_after.is_none(),
        baseline: plan.baseline.clone(),
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ResizeInput {
    pub(crate) floor: Decimal,
    pub(crate) largest_compliant_size: Option<Decimal>,
    pub(crate) quote: String,
    pub(crate) rule: String,
}

#[derive(Debug, Clone)]
pub(crate) struct SliceInput {
    pub(crate) market_order_cost: Decimal,
    pub(crate) sliced_cost: Decimal,
    pub(crate) slices: u32,
    pub(crate) window_minutes: u32,
    pub(crate) quote: String,
    pub(crate) baseline: String,
}

/// The guardian cheapest-safe unwind algorithm (R4), as a free function so it is
/// deterministic and unit-testable independent of any service state.
///
/// Objective: reach `recovery_target` at minimum total unwind cost, never
/// partially closing a structure into a worse residual, never touching protected
/// holdings. Selection is greedy by `delta_score / exit_cost`, re-evaluated after
/// each pick. `ProtectEth` applies a large penalty to ETH legs so they are chosen
/// only when nothing else can reach the target — and each such forced pick is
/// flagged `overrode_preference` for honest reporting.
pub(crate) fn guardian_cheapest_safe(
    candidates: &[UnwindCandidate],
    current_score: Decimal,
    recovery_target: Decimal,
    preference: GuardianPreference,
    emergency_slippage_reachable: bool,
) -> UnwindPlan {
    // Protected holdings and structure-breaking closes are vetoes, not
    // competitors — filtered out entirely before scoring (§4.1 vetoes multiply).
    let mut pool: Vec<&UnwindCandidate> = candidates
        .iter()
        .filter(|c| !c.protected && !c.breaks_structure_into_worse_residual)
        .collect();

    let mut score = current_score;
    let mut steps: Vec<UnwindStep> = Vec::new();
    let mut total_cost = Decimal::ZERO;
    let mut kept_eth = preference == GuardianPreference::ProtectEth;

    // Large ETH penalty under ProtectEth: expressed as a ranking demotion, not a
    // cost mutation, so reported costs stay truthful.
    let eth_penalty = Decimal::new(1_000_000, 0);

    while score < recovery_target && !pool.is_empty() {
        // Rank by delta_score / (exit_cost + penalty). Higher is better.
        // Guard against zero exit_cost with a tiny epsilon so a free close
        // sorts to the front without dividing by zero.
        let epsilon = Decimal::new(1, 6);
        let best_idx = pool
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| {
                let rank = |c: &UnwindCandidate| {
                    let penalty = if preference == GuardianPreference::ProtectEth && c.is_eth {
                        eth_penalty
                    } else {
                        Decimal::ZERO
                    };
                    // Prefer directional-reducing closes on ties via a small bonus.
                    let bonus = if c.reduces_directional_exposure {
                        Decimal::new(1, 3)
                    } else {
                        Decimal::ZERO
                    };
                    (c.delta_score + bonus) / (c.exit_cost + penalty + epsilon)
                };
                rank(a).cmp(&rank(b))
            })
            .map(|(i, _)| i);

        let Some(idx) = best_idx else { break };
        let chosen = pool.remove(idx);

        let overrode = preference == GuardianPreference::ProtectEth && chosen.is_eth;
        if overrode {
            kept_eth = false; // we were forced to touch ETH; report it honestly
        }

        score += chosen.delta_score;
        total_cost += chosen.exit_cost;
        steps.push(UnwindStep {
            label: chosen.label.clone(),
            delta_score: chosen.delta_score.normalize().to_string(),
            exit_cost: chosen.exit_cost.normalize().to_string(),
            overrode_preference: overrode,
        });
    }

    let reached = score >= recovery_target && emergency_slippage_reachable;

    let kept = match preference {
        GuardianPreference::ProtectEth if kept_eth => "your ETH stack".to_string(),
        GuardianPreference::ProtectEth => {
            "nothing of the position — protecting ETH was no longer enough to reach your floor"
                .to_string()
        }
        GuardianPreference::CheapestSafe => "nothing of the position".to_string(),
    };

    UnwindPlan {
        steps,
        cost_of_protection: total_cost.normalize().to_string(),
        reached_target: reached,
        resulting_score: score.normalize().to_string(),
        kept,
    }
}

/// Fixture-backed reporting used by the dev runtime and the test harness. Real
/// deployment swaps this for a service impl without changing tool signatures.
#[derive(Clone, Default)]
pub(crate) struct FixtureReporting;

impl Reporting for FixtureReporting {
    fn account_effect(&self, plan: &EffectPlan) -> AccountEffect {
        derive_account_effect(plan)
    }

    fn resize_solution(&self, input: &ResizeInput) -> ResizeSolution {
        ResizeSolution {
            floor: Figure::decimal(input.floor, input.quote.clone(), false),
            largest_compliant_size: input
                .largest_compliant_size
                .map(|size| Figure::decimal(size, input.quote.clone(), true)),
            rule: input.rule.clone(),
        }
    }

    fn exit_cost(&self, _position_id: &str) -> ExitCost {
        ExitCost {
            price_impact: Figure::estimate("0.31", "%"),
            time_to_flat_p90: Figure::estimate("4", "min"),
            net_result: Figure::estimate("104.20 on 100 committed", "USDT"),
            baseline: "simulated against the live book at quote time — an estimate, not a measured alternative".to_string(),
        }
    }

    fn slice_plan(&self, input: &SliceInput) -> SlicePlan {
        let saved = (input.market_order_cost - input.sliced_cost).normalize();
        let null_case = saved <= Decimal::ZERO;
        SlicePlan {
            market_order_cost: Figure::decimal(input.market_order_cost, input.quote.clone(), true),
            sliced_cost: Figure::decimal(input.sliced_cost, input.quote.clone(), true),
            // Never surface a negative "saving"; a null case reports $0 plainly.
            saved: Figure::decimal(
                if null_case { Decimal::ZERO } else { saved },
                input.quote.clone(),
                true,
            ),
            slices: input.slices,
            window_minutes: input.window_minutes,
            null_case,
            baseline: input.baseline.clone(),
        }
    }

    fn dollarpower(&self, _portfolio_id: &str) -> Dollarpower {
        Dollarpower {
            ratio: Figure::estimate("2.4", "×"),
            committed: Figure::estimate("10300", "USDT"),
            effective: Figure::estimate("24700", "USDT"),
        }
    }

    fn guardian_unwind(
        &self,
        candidates: &[UnwindCandidate],
        current_score: Decimal,
        recovery_target: Decimal,
        preference: GuardianPreference,
        emergency_slippage_reachable: bool,
    ) -> UnwindPlan {
        guardian_cheapest_safe(
            candidates,
            current_score,
            recovery_target,
            preference,
            emergency_slippage_reachable,
        )
    }

    fn carry_state(&self, _position_id: &str) -> CarryState {
        CarryState {
            position_id: _position_id.to_string(),
            entry_timestamp: "0".to_string(),
            negative_carry_window_days: 3,
            days_negative: 1,
            trigger_days: 3,
            avg_daily_carry: Figure::estimate("-0.31", "%"),
            fired: false,
            plan_executed: false,
            cadence_owner: "runtime",
        }
    }

    fn recommended_first_deposit(&self) -> RecommendedDeposit {
        RecommendedDeposit {
            amount: Figure::estimate("20", "USDT"),
            rationale: "clears transaction minimums".to_string(),
        }
    }

    fn demo_book(&self) -> Result<DemoBook, String> {
        Ok(fixture_demo_book(false))
    }
}

/// Canonical demo-book shape. `zero_edge` forces every carry/cost delta to 0 so
/// null-result copy can be tested without inventing a gain.
pub(crate) fn fixture_demo_book(zero_edge: bool) -> DemoBook {
    let z = |nonzero: &str, unit: &str| {
        if zero_edge {
            Figure::estimate("0", unit)
        } else {
            Figure::estimate(nonzero, unit)
        }
    };
    DemoBook {
        committed: Figure::estimate("100", "USDT"),
        borrowed: Figure::estimate("900", "USDT"),
        spot: Figure::estimate("1000", "USDT"),
        short: Figure::estimate("1000", "USDT"),
        borrow_apr: Figure::estimate("5.4", "%"),
        daily_carry_net: z("2.10", "USDT"),
        worst_week_daily: z("-4.80", "USDT"),
        negative_carry_close_days: Figure::estimate("3", "days"),
        dollarpower: Dollarpower {
            ratio: Figure::estimate("9.8", "×"),
            committed: Figure::estimate("100", "USDT"),
            effective: Figure::estimate("1000", "USDT"),
        },
        drill_move: Figure::estimate("-20", "%"),
        drill_step1_label: "Close the short".to_string(),
        drill_step1_freed: z("612", "USDT"),
        drill_step1_cost: z("1.40", "USDT"),
        drill_step2_label: "Repay the loan".to_string(),
        drill_step2_repay: Figure::estimate("900", "USDT"),
        drill_step2_cost: z("0.90", "USDT"),
        drill_total_cost: z("2.30", "USDT"),
        drill_seconds: Figure::estimate("40", "s"),
        rates_live: false,
    }
}

/// Reporting stand-in that returns a zero-edge demo book (null-result tests).
#[allow(dead_code)]
#[derive(Clone, Default)]
pub(crate) struct ZeroEdgeReporting;

impl Reporting for ZeroEdgeReporting {
    fn account_effect(&self, plan: &EffectPlan) -> AccountEffect {
        FixtureReporting.account_effect(plan)
    }
    fn resize_solution(&self, input: &ResizeInput) -> ResizeSolution {
        FixtureReporting.resize_solution(input)
    }
    fn exit_cost(&self, position_id: &str) -> ExitCost {
        FixtureReporting.exit_cost(position_id)
    }
    fn slice_plan(&self, input: &SliceInput) -> SlicePlan {
        FixtureReporting.slice_plan(input)
    }
    fn dollarpower(&self, portfolio_id: &str) -> Dollarpower {
        FixtureReporting.dollarpower(portfolio_id)
    }
    fn guardian_unwind(
        &self,
        candidates: &[UnwindCandidate],
        current_score: Decimal,
        recovery_target: Decimal,
        preference: GuardianPreference,
        emergency_slippage_reachable: bool,
    ) -> UnwindPlan {
        FixtureReporting.guardian_unwind(
            candidates,
            current_score,
            recovery_target,
            preference,
            emergency_slippage_reachable,
        )
    }
    fn carry_state(&self, position_id: &str) -> CarryState {
        FixtureReporting.carry_state(position_id)
    }
    fn recommended_first_deposit(&self) -> RecommendedDeposit {
        FixtureReporting.recommended_first_deposit()
    }
    fn demo_book(&self) -> Result<DemoBook, String> {
        Ok(fixture_demo_book(true))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cand(
        label: &str,
        delta: i64,
        cost: i64,
        breaks: bool,
        reduces: bool,
        protected: bool,
        is_eth: bool,
    ) -> UnwindCandidate {
        UnwindCandidate {
            label: label.to_string(),
            delta_score: Decimal::new(delta, 1),
            exit_cost: Decimal::new(cost, 0),
            breaks_structure_into_worse_residual: breaks,
            reduces_directional_exposure: reduces,
            protected,
            is_eth,
        }
    }

    // §4.1: a null-difference slice reports $0, never a fabricated saving.
    #[test]
    fn slice_null_case_reports_zero() {
        let r = FixtureReporting;
        let plan = r.slice_plan(&SliceInput {
            market_order_cost: Decimal::new(5, 2),
            sliced_cost: Decimal::new(5, 2),
            slices: 1,
            window_minutes: 0,
            quote: "USDT".to_string(),
            baseline: "book at quote time".to_string(),
        });
        assert!(plan.null_case);
        assert_eq!(plan.saved.value, "0");
    }

    #[test]
    fn transition_direction_for_rapv_is_higher_safer() {
        let fall = Transition::new(Decimal::new(7400, 0), Decimal::new(6800, 0), "RAPV", "rapv");
        assert_eq!(fall.direction, "less safe");
        let rise = Transition::new(Decimal::new(6800, 0), Decimal::new(7400, 0), "RAPV", "rapv");
        assert_eq!(rise.direction, "safer");
        let flat = Transition::new(Decimal::ONE, Decimal::ONE, "%", "yield");
        assert_eq!(flat.direction, "unchanged");
        assert!(flat.unchanged);
    }

    #[test]
    fn transition_direction_for_liquidation_score_is_lower_safer() {
        let fall = Transition::new(
            Decimal::new(38, 1),
            Decimal::new(21, 1),
            "",
            "liquidation_risk",
        );
        assert_eq!(fall.direction, "safer");
        assert_eq!(fall.before, "3.8");
        assert_eq!(fall.after, "2.1");
        let rise = Transition::new(
            Decimal::new(21, 1),
            Decimal::new(38, 1),
            "",
            "liquidation_risk",
        );
        assert_eq!(rise.direction, "less safe");
    }

    fn close_short_plan(risk_after: Option<Decimal>) -> EffectPlan {
        EffectPlan {
            exposure_symbol: "WBTC".to_string(),
            exposure_before: Decimal::new(270_785, 2),
            exposure_after: Decimal::ZERO,
            available_before: Decimal::new(49_115, 2),
            available_after: Decimal::new(330_891, 2),
            quote: "USDT".to_string(),
            liquidation_risk_before: Some(Decimal::new(38, 1)),
            liquidation_risk_after: risk_after,
            estimated_cost: Some(Decimal::new(840, 2)),
            missing_mark_symbols: Vec::new(),
            post_trade_risk_unavailable: risk_after.is_none(),
            concern_clause: "the open WBTC short was your main directional exposure".to_string(),
            baseline: "live account snapshot versus this intent".to_string(),
        }
    }

    #[test]
    fn derive_rail_matches_snapshot_plus_evaluation() {
        let effect = derive_account_effect(&close_short_plan(Some(Decimal::new(21, 1))));
        assert_eq!(effect.exposure_symbol, "WBTC");
        assert_eq!(effect.directional_exposure.before, "2707.85");
        assert_eq!(effect.directional_exposure.after, "0");
        assert_eq!(effect.available_to_deploy.before, "491.15");
        assert_eq!(effect.available_to_deploy.after, "3308.91");
        let risk = effect.liquidation_risk.expect("score present");
        assert_eq!(risk.before, "3.8");
        assert_eq!(risk.after, "2.1");
        assert_eq!(risk.direction, "safer");
        assert_eq!(effect.direction.as_deref(), Some("safer"));
        assert_eq!(
            effect.estimated_cost.as_ref().map(|f| f.value.as_str()),
            Some("8.4")
        );
        assert!(!effect.post_trade_risk_unavailable);
        assert!(effect.expected_net_yield.is_none());
    }

    #[test]
    fn derive_omits_risk_when_post_trade_unavailable() {
        let effect = derive_account_effect(&close_short_plan(None));
        assert!(effect.liquidation_risk.is_none());
        assert!(effect.direction.is_none());
        assert!(effect.post_trade_risk_unavailable);
        assert_eq!(effect.directional_exposure.after, "0");
        assert_eq!(
            effect.estimated_cost.as_ref().map(|f| f.value.as_str()),
            Some("8.4")
        );
    }

    // A "saving" can never be reported as negative.
    #[test]
    fn slice_never_reports_negative_saving() {
        let r = FixtureReporting;
        let plan = r.slice_plan(&SliceInput {
            market_order_cost: Decimal::new(10, 0),
            sliced_cost: Decimal::new(30, 0),
            slices: 4,
            window_minutes: 10,
            quote: "USDT".to_string(),
            baseline: "book".to_string(),
        });
        assert!(plan.null_case);
        assert_eq!(plan.saved.value, "0");
    }

    // R4: the algorithm stops exactly when the recovery target is reached.
    #[test]
    fn guardian_stops_at_recovery_target() {
        let candidates = vec![
            cand("close A", 8, 100, false, true, false, false), // +0.8
            cand("close B", 8, 100, false, true, false, false), // +0.8
            cand("close C", 8, 100, false, true, false, false), // +0.8
        ];
        let plan = guardian_cheapest_safe(
            &candidates,
            Decimal::new(60, 1), // 6.0
            Decimal::new(70, 1), // target 7.0
            GuardianPreference::CheapestSafe,
            true,
        );
        // Two closes (+1.6 → 7.6) reach 7.0; a third is unnecessary.
        assert_eq!(plan.steps.len(), 2);
        assert!(plan.reached_target);
    }

    // R4: protected holdings are vetoes — never chosen.
    #[test]
    fn guardian_never_touches_protected() {
        let candidates = vec![
            cand("protected vault", 50, 1, false, true, true, false),
            cand("plain leg", 20, 50, false, true, false, false),
        ];
        let plan = guardian_cheapest_safe(
            &candidates,
            Decimal::new(60, 1),
            Decimal::new(70, 1),
            GuardianPreference::CheapestSafe,
            true,
        );
        assert!(plan.steps.iter().all(|s| s.label != "protected vault"));
    }

    // R4: never partial-close a structure into a worse residual.
    #[test]
    fn guardian_refuses_worse_residual() {
        let candidates = vec![cand("break hedge", 90, 1, true, false, false, false)];
        let plan = guardian_cheapest_safe(
            &candidates,
            Decimal::new(60, 1),
            Decimal::new(70, 1),
            GuardianPreference::CheapestSafe,
            true,
        );
        assert!(plan.steps.is_empty());
        assert!(!plan.reached_target);
    }

    // R4: ProtectEth chooses a non-ETH path when one suffices, and keeps ETH.
    #[test]
    fn guardian_protects_eth_when_possible() {
        let candidates = vec![
            cand("close ETH", 20, 10, false, true, false, true),
            cand("close USDC leg", 20, 30, false, true, false, false),
        ];
        let plan = guardian_cheapest_safe(
            &candidates,
            Decimal::new(60, 1),
            Decimal::new(70, 1),
            GuardianPreference::ProtectEth,
            true,
        );
        // The single needed close should be the non-ETH leg despite higher cost.
        assert_eq!(plan.steps.len(), 1);
        assert_eq!(plan.steps[0].label, "close USDC leg");
        assert!(!plan.steps[0].overrode_preference);
        assert_eq!(plan.kept, "your ETH stack");
    }

    // R4: ProtectEth is forced onto ETH only when nothing else reaches target,
    // and reports the override honestly.
    #[test]
    fn guardian_reports_forced_eth_override() {
        let candidates = vec![
            cand("close ETH", 20, 10, false, true, false, true), // +2.0
            cand("close small", 2, 5, false, true, false, false), // +0.2
        ];
        // Need +1.0; small alone gives +0.2, so ETH must be used.
        let plan = guardian_cheapest_safe(
            &candidates,
            Decimal::new(60, 1),
            Decimal::new(70, 1),
            GuardianPreference::ProtectEth,
            true,
        );
        assert!(plan.reached_target);
        assert!(plan.steps.iter().any(|s| s.overrode_preference));
        assert!(plan.kept.contains("no longer enough"));
    }

    // Degraded state: target math met but emergency slippage unreachable →
    // reached_target is false (§4 degraded state: never override the limit).
    #[test]
    fn guardian_degraded_when_slippage_unreachable() {
        let candidates = vec![cand("close A", 20, 100, false, true, false, false)];
        let plan = guardian_cheapest_safe(
            &candidates,
            Decimal::new(60, 1),
            Decimal::new(70, 1),
            GuardianPreference::CheapestSafe,
            false, // emergency slippage NOT reachable
        );
        assert!(!plan.reached_target);
    }
}
