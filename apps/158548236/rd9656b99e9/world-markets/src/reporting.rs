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

use crate::size::ResolvedSize;

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
        let value = value.into();
        let unit = unit.into();
        Self {
            value: value.clone(),
            unit: unit.clone(),
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

    pub(crate) fn rendered(&self) -> String {
        crate::lookups::render_figure(&self.value, &self.unit, self.is_estimate)
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

    pub(crate) fn rendered_arrow(&self) -> String {
        let money = self.unit.eq_ignore_ascii_case("USDT") || self.unit == "$";
        if money {
            format!(
                "`{}` → `{}`",
                crate::lookups::format_money_str(&self.before, false),
                crate::lookups::format_money_str(&self.after, false)
            )
        } else if self.unit.is_empty() {
            format!(
                "`{}` → `{}`",
                trim_risk(&self.before),
                trim_risk(&self.after)
            )
        } else {
            format!("`{}` → `{}`", self.before, self.after)
        }
    }
}

fn trim_risk(raw: &str) -> String {
    crate::lookups::format_risk(raw)
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
    /// Pasteable concern line with the 0–10 delta, or empty when the delta is unprovable.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub(crate) concern_line: String,
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
    let concern = concern_line(liquidation_risk.as_ref(), &plan.concern_clause);
    let concern_clause = if liquidation_risk.is_some() {
        plan.concern_clause.clone()
    } else {
        String::new()
    };
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
        concern_clause,
        concern_line: concern,
        missing_mark_symbols: plan.missing_mark_symbols.clone(),
        post_trade_risk_unavailable: plan.post_trade_risk_unavailable
            || plan.liquidation_risk_after.is_none(),
        baseline: plan.baseline.clone(),
    }
}

fn concern_line(risk: Option<&Transition>, clause: &str) -> String {
    let Some(risk) = risk else {
        return String::new();
    };
    if risk.unchanged {
        return String::new();
    }
    let line = format!(
        "Risk `{}` → `{}`",
        crate::lookups::format_risk(&risk.before),
        crate::lookups::format_risk(&risk.after)
    );
    if clause.trim().is_empty() {
        line
    } else {
        format!("{line} — {clause}")
    }
}

/// House-formatted deny string. The raw engine `detail` stays beside this as `detail_rendered`.
pub(crate) fn render_deny(
    rule: &str,
    detail: &str,
    symbol: Option<&str>,
    projected: Option<Decimal>,
    cap_or_floor: Option<Decimal>,
    product_pair: Option<&str>,
) -> String {
    let money = |d: Decimal| format!("`{}`", crate::lookups::format_money(d, false));
    match rule {
        "position_notional" => {
            let pos = projected.map(money).unwrap_or_else(|| extract_money(detail));
            let cap = cap_or_floor.map(money).unwrap_or_else(|| extract_last_money(detail));
            let asset = symbol.unwrap_or("position");
            format!("⊘ That would take your {asset} position to {pos} — above your {cap} cap.")
        }
        "portfolio_floor" | "post_trade_portfolio_floor" => {
            let floor = cap_or_floor.map(money).unwrap_or_else(|| extract_last_money(detail));
            format!("⊘ That would take your portfolio below your floor — {floor}. The limit is yours, and it held.")
        }
        "market_not_permitted" => {
            let pair = product_pair.unwrap_or("that market");
            format!("⊘ `{pair}` isn't in your signed markets list. I can't trade it until you add it on World.")
        }
        "liquidatable" => {
            "⊘ Your account is eligible for liquidation and your mandate requires a halt. I'm not adding any exposure."
                .to_string()
        }
        "insufficient_spot_balance" => {
            let asset = symbol.unwrap_or("that asset");
            format!("⊘ That sell would move your live `{asset}` balance below zero.")
        }
        "withdraw_not_supported" => {
            "⊘ Withdrawal isn't a power the key has. Requests like this are rejected.".to_string()
        }
        "missing_mandate" | "unknown_mandate_key" | "invalid_mandate" | "unsupported_mandate_version" => {
            format!("⊘ {detail}")
        }
        "leverage" => {
            let cap = cap_or_floor
                .map(|d| format!("`{}×`", d.normalize()))
                .unwrap_or_else(|| extract_last_money(detail));
            format!("⊘ That leverage is above your {cap} cap.")
        }
        _ => {
            let cleaned = house_numbers(detail);
            format!("⊘ {cleaned}")
        }
    }
}

fn extract_money(detail: &str) -> String {
    crate::lookups::first_money_token(detail).unwrap_or_else(|| format!("`{detail}`"))
}

fn extract_last_money(detail: &str) -> String {
    crate::lookups::last_money_token(detail).unwrap_or_else(|| extract_money(detail))
}

fn house_numbers(detail: &str) -> String {
    crate::lookups::rewrite_engine_numbers(detail)
}

/// Frozen §6.5 receipt. The model pastes `message`; it does not compose fields.
pub(crate) fn render_receipt(
    happened: &str,
    why: &str,
    account_effect: Option<&AccountEffect>,
    fill_price: Option<&str>,
    cost: Option<&str>,
    policy: &str,
    next: &str,
    graduation: Option<&str>,
    landing_ledger: bool,
) -> String {
    let mut lines = vec![
        format!("What happened · {happened}"),
        format!("Why · {why}"),
    ];
    if let Some(effect) = account_effect {
        let mut bits = Vec::new();
        if !effect.directional_exposure.unchanged {
            bits.push(format!(
                "{} {}",
                effect.exposure_symbol,
                effect.directional_exposure.rendered_arrow()
            ));
        }
        if !effect.available_to_deploy.unchanged {
            bits.push(format!(
                "Available {}",
                effect.available_to_deploy.rendered_arrow()
            ));
        }
        if let Some(risk) = &effect.liquidation_risk
            && !risk.unchanged
        {
            bits.push(format!("Risk {}", risk.rendered_arrow()));
        }
        if let Some(cost_fig) = &effect.estimated_cost {
            bits.push(format!("Cost `{}`", cost_fig.rendered()));
        }
        if bits.is_empty() {
            bits.push("nothing measurable changed".to_string());
        }
        lines.push(format!("Account effect · {}", bits.join(" · ")));
    } else if let Some(price) = fill_price {
        lines.push(format!(
            "Account effect · fill `{price}` · cost `{cost}`",
            cost = cost.unwrap_or("—")
        ));
    } else {
        lines.push("Account effect · staged — fill pending the cancel window.".to_string());
    }
    if let Some(price) = fill_price {
        let cost_bit = cost.unwrap_or("—");
        lines.push(format!(
            "Execution quality · fill `{price}` · cost `{cost_bit}`."
        ));
    } else {
        lines.push("Execution quality · staged, not yet filled.".to_string());
    }
    lines.push(format!("Policy · {policy}"));
    let mut next_line = format!("Next · {next}");
    if landing_ledger {
        next_line.push_str(" · on your ledger");
    }
    lines.push(next_line);
    if let Some(effect) = account_effect
        && !effect.concern_line.is_empty()
    {
        lines.push(format!("One thing to flag: {}", effect.concern_line));
    }
    if let Some(grad) = graduation.filter(|s| !s.is_empty()) {
        lines.push(grad.to_string());
    }
    lines.push("[View on World ↗] [Explain] [Preview exit]".to_string());
    lines.join("\n")
}

pub(crate) const GRADUATION_NOTICE: &str =
    "Orders like this now execute automatically. Say `always ask` to keep confirmations.";

/// UNCLEAR (§6.21a) — non-trade register. Never assumes the user tried to buy.
pub(crate) const UNCLEAR_MESSAGE: &str = "I didn't catch that — I trade crypto spot, perps, and lending on World. Say what you'd like to do, or `/p` for positions.";

/// CONFIRM-ONCE (§6.4a) opt-out read-back. Live figures; not a request for yes.
pub(crate) fn render_confirm_once_readback(
    resolved: &ResolvedSize,
    asset: &str,
    product: &str,
) -> String {
    let dollars = crate::lookups::format_money(resolved.notional, false);
    let qty = crate::size::format_base_qty(resolved.base_qty);
    let mark = crate::lookups::format_mark_human(resolved.mark);
    format!(
        "Staging `{dollars}` of {asset} {product} — `~{qty}` {asset} at `~{mark}`.\nSends in 3s if you don't cancel.\n[Cancel]"
    )
}

/// RECEIPT / staged "What happened" — dollar size primary, ≤4-dp base qty parenthetical.
pub(crate) fn render_size_happened(
    resolved: &ResolvedSize,
    asset: &str,
    product: &str,
    fill_price: Option<&str>,
    staged: bool,
) -> String {
    let dollars = crate::lookups::format_money(resolved.notional, false);
    let qty = crate::size::format_qty_human(resolved.base_qty);
    let core = format!("`{dollars}` of {asset} {product} (~`{qty}` {asset})");
    if let Some(price) = fill_price.map(str::trim).filter(|p| !p.is_empty()) {
        format!("Sent {core}, filled at `{price}`.")
    } else if staged {
        format!("Staged {core} — 3s to fill.")
    } else {
        format!("Sent {core}.")
    }
}

/// CANT (§6.21) three-line wall. Category-level; never a trade clarification.
pub(crate) fn render_cant_wall(heard: &str, category: &str) -> String {
    let quoted = heard.trim().trim_end_matches('.');
    let display = match category {
        "food" => "meat or commodities",
        "that" => "that",
        other => other,
    };
    format!(
        "I heard \"{quoted}.\"\nWorld doesn't trade {display}.\nWorld trades crypto spot, perps, and lending."
    )
}

/// Dollarpower PASTE sentence. Operands: separate-venue (effective) ÷ World (committed).
pub(crate) fn render_dollarpower_message(dp: &Dollarpower) -> String {
    let committed = dp.committed.rendered();
    let effective = dp.effective.rendered();
    let ratio = dp.ratio.value.trim();
    format!(
        "Dollarpower is how hard each committed dollar works: separate-venue collateral `{effective}` ÷ World collateral `{committed}`. Yours is `{ratio}`× — your `{committed}` is doing the work of `{effective}`."
    )
}

pub(crate) fn render_protected_veto_message(asset: &str, absolute: bool) -> String {
    let asset = asset.trim().to_ascii_uppercase();
    let base = format!(
        "Stored: I'll avoid selling your {asset}. One exception you've already signed: if your portfolio breaches your floor and {asset} is the only way back above it, the guardian may sell some — your mandate outranks this preference. To make it absolute, change your policies on World."
    );
    if absolute {
        format!("{base} [View mandate on World ↗]")
    } else {
        base
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

    #[test]
    fn dollarpower_message_divides_effective_by_committed() {
        let dp = Dollarpower {
            ratio: Figure::estimate("2.4", "×"),
            committed: Figure::estimate("10300", "USDT"),
            effective: Figure::estimate("24700", "USDT"),
        };
        let message = render_dollarpower_message(&dp);
        assert!(message.contains("2.4"));
        assert!(message.contains("doing the work of"));
        let veto = render_protected_veto_message("SOL", true);
        assert!(veto.contains("guardian may sell"));
        assert!(veto.contains("View mandate on World"));
        assert!(!veto.to_lowercase().contains("i won't sell your sol"));
    }

    fn resolved_200_weth() -> ResolvedSize {
        use std::str::FromStr;
        ResolvedSize {
            input: "$200".into(),
            denomination: "quote",
            mark: Decimal::from_str("2500").unwrap(),
            base_qty: Decimal::from_str("0.0799427609831360745706074451").unwrap(),
            notional: Decimal::from(200),
            size: crate::size::Size::Quote(Decimal::from(200)),
        }
    }

    #[test]
    fn confirm_once_readback_has_live_figures_and_no_yes() {
        let message = render_confirm_once_readback(&resolved_200_weth(), "WETH", "spot");
        assert!(message.contains("$200"), "{message}");
        assert!(message.contains("WETH"));
        assert!(message.contains("spot"));
        assert!(message.contains("~"));
        assert!(message.contains("Sends in 3s if you don't cancel."));
        assert!(message.contains("[Cancel]"));
        let lower = message.to_ascii_lowercase();
        assert!(!lower.contains("yes"));
        assert!(!lower.contains("confirm to send"));
        assert!(!lower.contains("say yes"));
        let qty = message
            .split('`')
            .find(|t| t.starts_with('~') && t.chars().any(|c| c.is_ascii_digit()))
            .unwrap_or("");
        let frac = qty.trim_start_matches('~').split('.').nth(1).unwrap_or("");
        assert!(frac.len() <= 6, "{qty}");
    }

    #[test]
    fn size_happened_keeps_quantity_at_most_four_dp_and_shows_dollars() {
        let staged = render_size_happened(&resolved_200_weth(), "WETH", "spot", None, true);
        assert!(staged.contains("$200"), "{staged}");
        assert!(staged.contains("WETH"));
        let qty = staged
            .split('`')
            .find(|t| t.starts_with("0.") || t.parse::<f64>().is_ok() && t.contains('.'))
            .unwrap_or("");
        let frac = qty.split('.').nth(1).unwrap_or("");
        assert!(frac.len() <= 4, "qty token {qty} in {staged}");
        let filled =
            render_size_happened(&resolved_200_weth(), "WETH", "spot", Some("2465.71"), false);
        assert!(filled.contains("Sent"));
        assert!(filled.contains("filled at `2465.71`"));
        let receipt = render_receipt(
            &filled,
            "You asked to buy $200 of WETH.",
            None,
            Some("2465.71"),
            None,
            "within limits.",
            "Watching the fill.",
            Some(GRADUATION_NOTICE),
            true,
        );
        assert!(receipt.contains("$200"));
        assert!(receipt.contains(GRADUATION_NOTICE));
        let qty_in_receipt = receipt
            .split('`')
            .find(|t| *t == "0.0799" || t.starts_with("0.07"))
            .unwrap_or("");
        let frac = qty_in_receipt.split('.').nth(1).unwrap_or("");
        assert!(frac.len() <= 4, "{qty_in_receipt}");
    }

    #[test]
    fn cant_wall_is_three_lines_and_unclear_is_non_trade() {
        let wall = render_cant_wall("buy me $50 of beef", "food");
        assert!(wall.contains("I heard \"buy me $50 of beef.\""));
        assert!(wall.contains("World doesn't trade"));
        assert!(wall.contains("World trades crypto spot, perps, and lending."));
        assert!(!wall.to_ascii_lowercase().contains("say buy"));
        assert!(UNCLEAR_MESSAGE.contains("I trade crypto spot, perps, and lending"));
        assert!(UNCLEAR_MESSAGE.contains("/p"));
        assert!(!UNCLEAR_MESSAGE.to_ascii_lowercase().contains("say buy"));
    }
}
