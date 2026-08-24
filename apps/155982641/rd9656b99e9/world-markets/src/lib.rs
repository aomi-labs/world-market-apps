use aomi_sdk::*;

mod carry;
mod client;
mod execution;
mod guest;
mod liquidation_risk;
mod loans;
mod lookups;
mod mandate;
mod pnl;
mod preamble;
mod rates;
mod reporting;
mod tool;

dyn_aomi_app!(
    app = tool::WorldMarketsApp,
    name = "world-markets",
    version = "0.4.0",
    preamble = preamble::COMPOSED,
    tools = [
        tool::ListWorldAssets,
        tool::GetWorldAccount,
        tool::GetWorldMarket,
        tool::GetWorldRates,
        tool::GetWorldLoans,
        tool::PreviewWorldTrade,
        tool::CheckWorldMandate,
        tool::ExecuteWorldOrder,
        tool::CancelWorldOrder,
        tool::ExecuteWorldSwap,
        tool::RenewWorldLoans,
        tool::PayWorldLoanInterest,
        tool::CloseWorldLoan,
        tool::GetWorldAgentPermission,
        tool::GetWorldOpenOrders,
        tool::GetWorldPnl,
        tool::PreviewAccountEffect,
        tool::ComputeResize,
        tool::PreviewExit,
        tool::PlanLargeOrder,
        tool::GetDollarpower,
        tool::SimulateGuardianUnwind,
        tool::CheckNegativeCarry,
        tool::RenderShare,
        tool::RenderGuestSurface,
        tool::ApplyGuestUpgrade,
    ],
    namespaces = ["evm-core"],
    skill = {
        id: "world-markets/trading",
        sections: {
            instructions: "skill/instructions.md",
            lookups: "skill/lookups.md",
            workflows: "skill/workflows.md",
            action_rules: "skill/action-rules.md",
            safety: "skill/safety.md",
            atlas: "skill/reference/atlas.md",
            products: "skill/reference/products.md",
            account_model: "skill/reference/account-model.md",
            venue: "skill/reference/venue.md",
            dollarpower: "skill/reference/dollarpower.md",
            guardian: "skill/reference/guardian.md",
            notifications: "skill/reference/notifications.md",
            strategy_brain: "skill/reference/strategy-brain.md",
        },
    }
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composed_preamble_includes_lookup_rules() {
        assert!(
            preamble::COMPOSED.contains("Terse lookups"),
            "composed preamble must include instructions lookups section"
        );
        assert!(
            preamble::COMPOSED.contains("Portfolio"),
            "composed preamble must include balance lookup format"
        );
        assert!(
            preamble::COMPOSED.len() > preamble::ROLE_LEN + 5000,
            "composed preamble must embed skill sections for aomi-run"
        );
    }

    #[test]
    fn app_skill_is_valid_and_mandate_aware() {
        let skill = tool::WorldMarketsApp::default()
            .skill()
            .expect("World Markets must ship its app-scoped skill");

        assert_eq!(skill.id, "world-markets/trading");
        assert_eq!(
            skill
                .sections
                .iter()
                .map(|section| section.name.as_str())
                .collect::<Vec<_>>(),
            [
                "instructions",
                "lookups",
                "workflows",
                "action_rules",
                "safety",
                "atlas",
                "products",
                "account_model",
                "venue",
                "dollarpower",
                "guardian",
                "notifications",
                "strategy_brain",
            ]
        );
        assert!(skill.guard.is_none());
        assert!(skill.hooks.is_empty());
        skill
            .validate("world-markets")
            .expect("embedded app skill must satisfy the SDK contract");
    }
}
