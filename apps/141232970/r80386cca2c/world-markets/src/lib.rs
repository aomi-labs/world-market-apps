use aomi_sdk::*;

mod brain;
mod cant;
mod carry;
mod chart;
mod client;
mod execution;
mod guest;
mod liquidation_risk;
mod loans;
mod lookups;
mod mandate;
mod marketdata;
pub mod mini_app;
mod order_intent;
mod pnl;
mod preamble;
mod rates;
mod reporting;
mod research;
mod rpc;
mod share;
mod size;
mod speech_ontology;
mod staged;
mod stt;
mod tasks;
mod tool;
mod voice;
mod warm;

dyn_aomi_app!(
    app = tool::WorldMarketsApp,
    name = "world-markets",
    version = "0.4.0",
    preamble = preamble::COMPOSED,
    tools = [
        tool::ListWorldAssets,
        tool::GetWorldAccount,
        tool::RenderLookup,
        tool::WarmAccount,
        tool::GetHealthSnapshot,
        tool::GetStrategySnapshot,
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
        tool::RenderMarketChart,
        tool::RefreshMarketUniverse,
        tool::ClearMarketCharts,
        tool::GetWorldResearch,
        tool::GetWorldTasks,
        tool::SetWorldWatch,
        tool::SetWorldPreference,
        tool::CancelWorldTask,
        tool::PauseWorldWatch,
        tool::ResumeWorldWatch,
        tool::DrainWorldOutbound,
        tool::RecordWorldCorrection,
        tool::SetWorldConsent,
        tool::CloseWorldEpisode,
    ],
    secrets = [tool::MARKET_DATA_API_KEY],
    namespaces = ["evm-core"],
    skill = {
        id: "world-markets/trading",
        sections: {
            instructions: "skill/instructions.md",
            lookups: "skill/lookups.md",
            workflows: "skill/workflows.md",
            action_rules: "skill/action-rules.md",
            exemplars: "skill/exemplars.md",
            safety: "skill/safety.md",
            atlas: "skill/reference/atlas.md",
            products: "skill/reference/products.md",
            account_model: "skill/reference/account-model.md",
            venue: "skill/reference/venue.md",
            dollarpower: "skill/reference/dollarpower.md",
            guardian: "skill/reference/guardian.md",
            notifications: "skill/reference/notifications.md",
            strategy_brain: "skill/reference/strategy-brain.md",
            turn_contract: "skill/turn-contract.md",
        },
    }
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composed_preamble_includes_lookup_rules() {
        let header = preamble::ROLE_HEADER_FOR_TEST;
        assert!(
            header.contains("precise financial operator"),
            "role header must name the operator"
        );
        assert!(
            header.contains("turn contract"),
            "role header must point at the turn contract"
        );
        assert!(
            !header.contains("Terse lookups")
                && !header.contains("render_lookup")
                && !header.contains("render_market_chart")
                && !header.contains("clear_market_charts"),
            "role header must not carry tool names or token-dispatch rules"
        );

        let lookups = include_str!("skill/lookups.md");
        assert!(
            preamble::COMPOSED.contains(lookups),
            "composed preamble must embed lookups.md after the role header"
        );
        assert!(
            lookups.contains("whole-message match only") || lookups.contains("whole intent"),
            "lookups.md must carry the terse-token dispatch"
        );
        assert!(
            lookups.contains("cancel task") && lookups.contains("Lone `d` is dollarpower"),
            "lookups.md must carry the relocated chart/cancel dispatch"
        );
        assert!(
            preamble::COMPOSED.contains("Portfolio"),
            "composed preamble must include balance lookup format"
        );
        assert!(
            preamble::COMPOSED.contains("open_instructions"),
            "composed preamble must tell the agent to load ledger open_instructions"
        );
        assert!(
            preamble::COMPOSED.contains("exemplars.md")
                || preamble::COMPOSED.contains("# Exemplars"),
            "composed preamble must include exemplars"
        );
        assert!(
            preamble::COMPOSED.contains("# Turn contract"),
            "composed preamble must include the turn contract"
        );
        assert!(
            preamble::COMPOSED.len() > preamble::ROLE_LEN + 5000,
            "composed preamble must embed skill sections for aomi-run"
        );
    }

    #[test]
    fn hosted_skill_sections_match_composed_core_and_end_on_turn_contract() {
        // Allowed differences, listed so this test cannot silently accept new drift:
        // - role header: COMPOSED-only (`ROLE_HEADER`)
        // - guest.md / share.md: COMPOSED-only; hosted omits them (pre-existing).
        //   Flag: Telegram is where start=g_/start=ref_ guests arrive — owner to
        //   confirm whether guest copy is composed elsewhere hosted-side.
        let skill = tool::WorldMarketsApp::default()
            .skill()
            .expect("World Markets must ship its app-scoped skill");
        let hosted: Vec<&str> = skill
            .sections
            .iter()
            .map(|section| section.name.as_str())
            .collect();
        assert_eq!(hosted.as_slice(), preamble::HOSTED_SKILL_SECTION_NAMES);
        assert_eq!(
            &hosted[..hosted.len() - 1],
            preamble::SHARED_CORE_SECTION_NAMES,
            "hosted core must match COMPOSED's shared core (instructions…strategy_brain)"
        );
        assert_eq!(
            hosted.last().copied(),
            Some("turn_contract"),
            "turn-contract.md must be last in the hosted section list"
        );

        let contract = include_str!("skill/turn-contract.md").trim_end();
        assert!(
            preamble::COMPOSED.trim_end().ends_with(contract),
            "turn-contract.md must be the final section of COMPOSED"
        );
        assert!(
            preamble::COMPOSED.contains(include_str!("skill/exemplars.md")),
            "COMPOSED must include exemplars.md after action-rules"
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
            preamble::HOSTED_SKILL_SECTION_NAMES.to_vec()
        );
        assert!(skill.guard.is_none());
        assert!(skill.hooks.is_empty());
        skill
            .validate("world-markets")
            .expect("the hosted World Markets skill must fit the runtime budget");
    }
}
