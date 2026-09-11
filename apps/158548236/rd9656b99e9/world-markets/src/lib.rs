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
        tool::ProjectWorldAnswer,
        tool::SetWorldConsent,
        tool::CloseWorldEpisode,
    ],
    secrets = [
        tool::MARKET_DATA_API_KEY,
        tool::TELEGRAM_BOT_TOKEN,
        tool::WORLD_MINI_APP_URL,
    ],
    namespaces = ["evm-core"],
    skills = [
        {
            id: "world-markets/trading",
            description: "World Markets account context, mandate-aware trading rules, and terse lookups.",
            sections: {
                instructions: "skill/instructions.md",
                lookups: "skill/lookups.md",
            },
        },
        {
            id: "world-markets/execution",
            description: "World order previews, atomic execution, receipts, blocks, guardian and carry workflows.",
            sections: { workflows: "skill/workflows.md" },
        },
        {
            id: "world-markets/monitoring",
            description: "World standing instructions, health, research, watches, tasks, and advisory workflows.",
            sections: { workflows_monitoring: "skill/workflows-monitoring.md" },
        },
        {
            id: "world-markets/reporting",
            description: "World response formats, action rules, examples, and trading safety requirements.",
            sections: {
                action_rules: "skill/action-rules.md",
                exemplars: "skill/exemplars.md",
                safety: "skill/safety.md",
            },
        },
        {
            id: "world-markets/reference",
            description: "World venue, account and risk concepts, notifications, and the final turn contract.",
            sections: {
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
        },
    ]
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_preamble_reserves_space_for_backend_harness() {
        let manifest = tool::WorldMarketsApp::default().manifest();
        // The 32 KB cap applies AFTER compose_preamble adds the harness, EVM
        // account context and provider profile. Reserve 20 KB for that host text;
        // comparing the raw manifest to 32 KB missed a real startup failure.
        assert!(
            manifest.preamble.len() + 20_000 <= 32_000,
            "preamble is {} bytes",
            manifest.preamble.len()
        );
    }

    #[test]
    fn every_tool_schema_is_openai_strict_compatible() {
        /// Does this node declare itself an object? `["object", "null"]` is the
        /// nullable spelling and carries the same obligations.
        fn is_object(object: &serde_json::Map<String, serde_json::Value>) -> bool {
            let declared = match object.get("type") {
                Some(serde_json::Value::String(name)) => name == "object",
                Some(serde_json::Value::Array(names)) => {
                    names.iter().any(|name| name.as_str() == Some("object"))
                }
                _ => false,
            };
            declared || object.contains_key("properties")
        }

        fn walk(node: &serde_json::Value, path: &str, offenses: &mut Vec<String>) {
            let Some(object) = node.as_object() else {
                // A bare `true` is JSON Schema's "anything" — the other
                // rendering of an untyped field, and equally rejected.
                if node.is_boolean() && !path.ends_with("additionalProperties") {
                    offenses.push(format!("{path}: bare boolean schema"));
                }
                return;
            };

            let typed = ["type", "$ref", "anyOf", "oneOf", "allOf"]
                .iter()
                .any(|key| object.contains_key(*key));
            if !path.is_empty() && !typed {
                offenses.push(format!("{path}: no `type` (declare the field's shape)"));
            }

            // Absent is fine — `rig` fills in `false`. Anything else is sent
            // as-is and strict mode rejects it.
            if is_object(object)
                && let Some(extra) = object.get("additionalProperties")
                && extra != &serde_json::Value::Bool(false)
            {
                offenses.push(format!(
                    "{path}.additionalProperties: {extra} (strict mode admits only `false`; \
                     name the fields instead of using a map)"
                ));
            }

            for (key, value) in object {
                let child = |segment: &str| {
                    if path.is_empty() {
                        segment.to_string()
                    } else {
                        format!("{path}.{segment}")
                    }
                };
                match key.as_str() {
                    "properties" | "$defs" | "definitions" => {
                        for (name, entry) in value.as_object().into_iter().flatten() {
                            walk(entry, &child(&format!("{key}.{name}")), offenses);
                        }
                    }
                    "items" | "additionalProperties" => walk(value, &child(key), offenses),
                    "anyOf" | "oneOf" | "allOf" => {
                        for (index, entry) in value.as_array().into_iter().flatten().enumerate() {
                            walk(entry, &child(&format!("{key}[{index}]")), offenses);
                        }
                    }
                    _ => {}
                }
            }
        }

        let app = tool::WorldMarketsApp::default();
        for tool in app.tools() {
            let mut offenses = Vec::new();
            walk(&tool.parameters_schema, "", &mut offenses);
            assert!(
                offenses.is_empty(),
                "{} is not strict-compatible:\n  {}",
                tool.name,
                offenses.join("\n  ")
            );
        }
    }

    #[test]
    fn every_tool_schema_has_explicit_object_properties() {
        fn check(schema: &serde_json::Value, path: &str) {
            match schema {
                serde_json::Value::Object(fields) => {
                    if fields.get("type").is_some_and(|kind| {
                        kind == "object"
                            || kind
                                .as_array()
                                .is_some_and(|types| types.iter().any(|kind| kind == "object"))
                    }) {
                        assert!(
                            fields
                                .get("properties")
                                .is_some_and(|value| value.is_object()),
                            "provider-facing object schema needs properties: {path}"
                        );
                    }
                    for (name, value) in fields {
                        check(value, &format!("{path}/{name}"));
                    }
                }
                serde_json::Value::Array(values) => {
                    for (index, value) in values.iter().enumerate() {
                        check(value, &format!("{path}/{index}"));
                    }
                }
                _ => {}
            }
        }

        let manifest = tool::WorldMarketsApp::default().manifest();
        assert_eq!(manifest.sdk_version, "5.0.0");
        assert!(!manifest.tools.is_empty());
        for tool in manifest.tools {
            check(&tool.parameters_schema, &tool.name);
        }
    }

    #[test]
    fn preamble_keeps_safety_and_routes_detailed_operating_rules() {
        assert!(preamble::ROLE_HEADER_FOR_TEST.contains("precise financial operator"));
        let skills = tool::WorldMarketsApp::default().skills();
        for name in ["instructions", "lookups", "action_rules", "exemplars"] {
            let section = skills
                .iter()
                .flat_map(|skill| &skill.sections)
                .find(|section| section.name == name)
                .expect("operating rules remain available");
            assert!(
                !preamble::COMPOSED.contains(&section.content),
                "detailed rules must not consume the startup preamble budget"
            );
        }
        for content in [
            include_str!("skill/safety.md"),
            include_str!("skill/guest.md"),
            include_str!("skill/share.md"),
        ] {
            assert!(
                preamble::COMPOSED.contains(content),
                "always-active policy missing"
            );
        }
        assert!(preamble::COMPOSED.contains("Before any World tool call"));
        assert!(preamble::COMPOSED.contains("Skill activation precedes"));
        assert!(preamble::COMPOSED.contains("again each serve cycle"));
    }

    #[test]
    fn hosted_skill_sections_match_composed_core_and_end_on_turn_contract() {
        // Allowed differences, listed so this test cannot silently accept new drift:
        // - role header: COMPOSED-only (`ROLE_HEADER`)
        // - guest.md / share.md: COMPOSED-only; hosted omits them (pre-existing).
        //   Flag: Telegram is where start=g_/start=ref_ guests arrive — owner to
        //   confirm whether guest copy is composed elsewhere hosted-side.
        let skills = tool::WorldMarketsApp::default().skills();
        let hosted: Vec<&str> = skills
            .iter()
            .flat_map(|skill| &skill.sections)
            .map(|section| section.name.as_str())
            .collect();
        assert_eq!(hosted.as_slice(), preamble::HOSTED_SKILL_SECTION_NAMES);
        assert_eq!(
            &hosted[..hosted.len() - 1],
            preamble::SHARED_CORE_SECTION_NAMES,
            "all detailed workflow and reference sections must remain in the skill catalog"
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
    }

    #[test]
    fn app_skill_is_valid_and_mandate_aware() {
        let skills = tool::WorldMarketsApp::default().skills();

        assert_eq!(skills.len(), 5);
        assert_eq!(skills[0].id, "world-markets/trading");
        assert_eq!(
            skills
                .iter()
                .flat_map(|skill| &skill.sections)
                .map(|section| section.name.as_str())
                .collect::<Vec<_>>(),
            preamble::HOSTED_SKILL_SECTION_NAMES.to_vec()
        );
        aomi_sdk::validate_app_skills("world-markets", &skills)
            .expect("all shipped skills must pass the same validation as the backend loader");
        for skill in &skills {
            assert!(skill.guard.is_none());
            assert!(skill.hooks.is_empty());
            assert!(
                preamble::COMPOSED.contains(&skill.id),
                "preamble must route detailed work to {}",
                skill.id
            );
        }
    }
}
