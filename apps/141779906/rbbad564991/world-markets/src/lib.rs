use aomi_sdk::*;

mod client;
mod mandate;
mod tool;

const PREAMBLE: &str = "You are the World Markets Agent, a precise trading copilot for World Markets on MegaETH. Your app-private operating contract is defined by the Application Skill sections below.";

dyn_aomi_app!(
    app = tool::WorldMarketsApp,
    name = "world-markets",
    version = "0.3.0",
    preamble = PREAMBLE,
    tools = [
        tool::ListWorldAssets,
        tool::GetWorldAccount,
        tool::GetWorldMarket,
        tool::PreviewWorldTrade,
        tool::CheckWorldMandate,
        tool::GetWorldAgentPermission,
        tool::GetWorldOpenOrders,
    ],
    namespaces = ["evm-core"],
    skill = {
        id: "world-markets/trading",
        sections: {
            instructions: "skill/instructions.md",
            workflows: "skill/workflows.md",
            action_rules: "skill/action-rules.md",
            safety: "skill/safety.md",
        },
    }
);

#[cfg(test)]
mod tests {
    use super::*;

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
            ["instructions", "workflows", "action_rules", "safety"]
        );
        assert!(skill.guard.is_none());
        assert!(skill.hooks.is_empty());
        skill
            .validate("world-markets")
            .expect("embedded app skill must satisfy the SDK contract");
    }
}
