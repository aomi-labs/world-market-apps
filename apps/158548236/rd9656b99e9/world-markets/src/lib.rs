use aomi_sdk::*;

mod book;
mod client;
mod liquidation_risk;
mod loans;
mod lookups;
mod mandate;
mod order_intent;
mod order_word;
mod pnl;
mod preamble;
mod rates;
mod rpc;
mod size;
mod tool;

dyn_aomi_app!(
    app = tool::WorldMarketsApp,
    name = "world-markets",
    version = "0.4.0",
    preamble = preamble::COMPOSED,
    tools = [
        tool::ListWorldAssets,
        tool::GetWorldAccount,
        tool::GetHealthSnapshot,
        tool::GetWorldMarket,
        tool::GetWorldRates,
        tool::GetWorldLoans,
        tool::GetWorldOpenOrders,
        tool::PreviewWorldTrade,
        tool::CheckWorldMandate,
        tool::GetWorldPnl,
        tool::ComputeResize,
        tool::WorldPackOrder,
        tool::WorldResolveBook,
    ],
    namespaces = ["evm-core"],
    skills = [
        {
            id: "world-markets/trading",
            description: "World Markets account model, mandate rules, and terse lookups as ordinary tool calls.",
            sections: { trading: "skill/trading.md" },
        },
        {
            id: "world-markets/execution",
            description: "Activate together with trading in the first pass for every World action, before preview; omit reporting to fit the shared activation budget. Procedure: allow verdict → resolve book → pack order word → evm_stage_tx → simulate_batch → evm_commit_txs, guarded to the exchange contract.",
            sections: { workflow: "skill/execution.md" },
            guard: "skill/guard.json",
        },
        {
            id: "world-markets/reporting",
            description: "World response formats: honest numbers, deny copy per rule, the mandate-absent handshake, and trading safety.",
            sections: { reporting: "skill/reporting.md" },
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
        assert_eq!(manifest.sdk_version, "5.0.1");
        assert!(!manifest.tools.is_empty());
        for tool in manifest.tools {
            check(&tool.parameters_schema, &tool.name);
        }
    }

    #[test]
    fn preamble_keeps_safety_and_routes_to_the_three_skills() {
        assert!(preamble::ROLE_HEADER_FOR_TEST.contains("precise financial operator"));
        let app = tool::WorldMarketsApp::default();
        for skill in app.skills() {
            assert!(
                preamble::COMPOSED.contains(&skill.id),
                "preamble must route detailed work to {}",
                skill.id
            );
            for section in &skill.sections {
                assert!(
                    !preamble::COMPOSED.contains(&section.content),
                    "detailed rules must not consume the startup preamble budget"
                );
            }
        }
        assert!(preamble::COMPOSED.contains(include_str!("skill/safety.md")));
        let contract = include_str!("skill/turn-contract.md").trim_end();
        assert!(
            preamble::COMPOSED.trim_end().ends_with(contract),
            "turn-contract.md must be the final section of COMPOSED"
        );
        assert!(preamble::COMPOSED.contains("Before any World tool call"));
        assert!(preamble::COMPOSED.contains("one activate_skills call in the first pass"));
        assert!(preamble::COMPOSED.contains("before previewing the intent"));
        assert!(!preamble::COMPOSED.contains("also activate"));
        assert!(preamble::COMPOSED.contains("again each serve cycle"));
        for banned in ["Telegram", "sidecar", "Mini App", "render_lookup"] {
            assert!(
                !preamble::COMPOSED.contains(banned),
                "preamble must not mention {banned}"
            );
        }
    }

    #[test]
    fn manifest_lists_the_thirteen_tools_without_secrets() {
        let app = tool::WorldMarketsApp::default();
        let manifest = app.manifest();
        let names: Vec<&str> = manifest.tools.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "list_world_assets",
                "get_world_account",
                "get_health_snapshot",
                "get_world_market",
                "get_world_rates",
                "get_world_loans",
                "get_world_open_orders",
                "preview_world_trade",
                "check_world_mandate",
                "get_world_pnl",
                "compute_resize",
                "world_pack_order",
                "world_resolve_book",
            ]
        );
        assert!(app.secrets().is_none(), "the app declares no secrets");
        assert_eq!(app.namespaces(), Some(vec!["evm-core".to_string()]));
    }

    #[test]
    fn routed_skill_pairs_fit_host_activation_budget() {
        let skills = tool::WorldMarketsApp::default().skills();
        for pair in [
            ["world-markets/trading", "world-markets/execution"],
            ["world-markets/trading", "world-markets/reporting"],
        ] {
            // Match aomi-skills::estimate_activation_tokens: app skills have
            // no Tools metadata and use render_sections as instruction_md.
            let tokens: usize = pair
                .iter()
                .map(|id| {
                    let skill = skills.iter().find(|skill| skill.id == *id).unwrap();
                    format!(
                        "## Skill: {}\n\n{}",
                        skill.id,
                        skill.render_sections().trim_end()
                    )
                    .chars()
                    .count()
                    .div_ceil(4)
                })
                .sum();
            assert!(tokens <= 4000, "{pair:?} uses {tokens} activation tokens");
        }
        assert!(
            preamble::COMPOSED
                .contains("select exactly world-markets/trading and world-markets/execution")
        );
        assert!(
            preamble::COMPOSED
                .contains("select exactly world-markets/trading and world-markets/reporting")
        );
    }

    #[test]
    fn app_skills_are_valid_and_only_execution_carries_the_guard() {
        let skills = tool::WorldMarketsApp::default().skills();
        let ids: Vec<&str> = skills.iter().map(|skill| skill.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "world-markets/trading",
                "world-markets/execution",
                "world-markets/reporting"
            ]
        );
        aomi_sdk::validate_app_skills("world-markets", &skills)
            .expect("all shipped skills must pass the same validation as the backend loader");
        for skill in &skills {
            assert!(skill.hooks.is_empty(), "{} binds no host hooks", skill.id);
            assert!(
                skill.est_tokens() <= aomi_sdk::APP_SKILL_TOKEN_BUDGET,
                "{} is {} tokens",
                skill.id,
                skill.est_tokens()
            );
            assert_eq!(skill.sections.len(), 1);
            assert_eq!(
                skill.guard.is_some(),
                skill.id == "world-markets/execution",
                "only the execution skill carries a guard table"
            );
        }
    }

    #[test]
    fn guard_json_covers_trading_selectors_only() {
        let skills = tool::WorldMarketsApp::default().skills();
        let execution = skills
            .iter()
            .find(|skill| skill.id == "world-markets/execution")
            .unwrap();
        let guard = execution.guard.as_ref().expect("execution guard");
        assert_eq!(guard.id, "world-markets/execution");
        assert!(guard.svm.is_none());
        let evm = guard.evm.as_ref().expect("evm guard");
        assert_eq!(
            evm.contracts.get("EXCHANGE").map(String::as_str),
            Some("0xf6b54e033bb45a583aa642924bcef78b804588ae")
        );
        assert_eq!(evm.contracts.len(), 1);
        assert_eq!(evm.allowed_contracts, ["EXCHANGE"]);
        assert_eq!(evm.approve_spenders, ["EXCHANGE"]);
        assert_eq!(evm.chain_ids, [2092151908]);

        let expected = [
            "newSpotBuyOrder(address,uint256)",
            "newSpotSellOrder(address,uint256)",
            "newPerpBuyOrder(address,uint256)",
            "newPerpSellOrder(address,uint256)",
            "newLendOrder(address,uint256)",
            "newBorrowOrder(address,uint256)",
            "cancelSpotBuyOrder(address,uint256)",
            "cancelSpotSellOrder(address,uint256)",
            "cancelPerpBuyOrder(address,uint256)",
            "cancelPerpSellOrder(address,uint256)",
            "cancelLendOrder(address,uint256)",
            "cancelBorrowOrder(address,uint256)",
            "renewLoan(uint64,uint64,uint64)",
            "payInterestAndFees(uint64,uint64,bool)",
        ];
        let mut declared: Vec<&str> = evm.selectors.values().map(String::as_str).collect();
        declared.sort_unstable();
        let mut wanted = expected.to_vec();
        wanted.sort_unstable();
        assert_eq!(declared, wanted, "exactly the fourteen trading functions");
        let mut allowed: Vec<&str> = evm.allowed_selectors.iter().map(String::as_str).collect();
        allowed.sort_unstable();
        let mut keys: Vec<&str> = evm.selectors.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            allowed, keys,
            "every declared selector is allowed, nothing else"
        );
        for signature in expected {
            aomi_sdk::resolve_selector(signature).expect(signature);
        }
        for forbidden in [
            "batchCommands(uint64,uint256[])",
            "liquidate(uint64,uint64,uint64)",
            "bankruptcy(uint64,uint16)",
        ] {
            assert!(!declared.contains(&forbidden), "{forbidden} must stay out");
        }
    }
}
