//! The agent registry: which agents exist, which protocols each speaks, and the
//! custom agents the user added.

use crate::vm::e2s;
use kiwanod::api::agents as daemon;
use kiwanod::store::Store;

pub const AGENTS: [(&str, &str); 26] = [
    ("claude", "Claude Code"),
    ("codex", "Codex"),
    ("gemini", "Gemini CLI"),
    ("grokbuild", "Grok Build"),
    ("claude-desktop", "Claude Desktop"),
    ("opencode", "OpenCode"),
    ("openclaw", "OpenClaw"),
    ("hermes", "Hermes"),
    ("pi", "Pi"),
    ("omp", "oh-my-pi"),
    ("commandcode", "Command Code"),
    ("dsh", "DeepSeek Harness"),
    ("hanaagent", "HanaAgent"),
    ("workbuddy", "WorkBuddy"),
    ("codebuddy", "CodeBuddy Code"),
    ("kimi", "Kimi Code CLI"),
    ("qwen", "Qwen Code"),
    ("cline", "Cline"),
    ("mimo", "MiMo Code"),
    ("mcode", "MiniMax Code"),
    ("aider", "Aider"),
    ("continue", "Continue"),
    ("crush", "Crush"),
    ("droid", "Droid"),
    ("goose", "Goose"),
    ("zcode", "ZCode"),
];

/// Whether `id` names a built-in agent — one whose *config* this app knows how
/// to rewrite or detect.
///
/// The distinction matters and is easy to blur: everything that reaches into an
/// agent's files (`takeover`, `creds`, `import`, `detect`) must only ever see a
/// built-in, while anything that *lists* agents (the Apps segments, the
/// dashboard's breakdowns, the provider dialog's multi-select) shows built-ins
/// and user-defined ones together.
pub fn is_builtin_agent(id: &str) -> bool {
    AGENTS.iter().any(|(a, _)| *a == id)
}

// Re-exported, not declared here: the daemon serves these types now, so they
// live where both sides can see them. The paths are unchanged on purpose.
pub use kiwano_api::agents::{CustomAgentVm, PromptLatencyVm};

/// Every agent the UI should offer, built-ins first (registry order), then the
/// user's own in the order they were created.
pub(crate) fn list_agents(store: &Store) -> Result<Vec<(String, String)>, String> {
    let mut out: Vec<(String, String)> = AGENTS
        .iter()
        .map(|(id, label)| (id.to_string(), label.to_string()))
        .collect();
    for a in store.list_custom_agents().map_err(e2s)? {
        out.push((a.id, a.label));
    }
    Ok(out)
}

/// One prompt round trip against a provider — served by the daemon
/// (`kiwanod::api::probe::test_provider_latency`). The verdict it writes is the
/// health row the Status column reads, which the daemon owns.
pub async fn test_provider_latency(store: &Store, id: &str) -> Result<PromptLatencyVm, String> {
    kiwanod::api::probe::test_provider_latency(store, id)
        .await
        .map_err(|e| e.to_string())
}

/// Create a user-defined agent — served by the daemon
/// (`kiwanod::api::agents::create_custom_agent`).
///
/// This wrapper mints the id itself so the signature every existing caller knows
/// stays put: it is the **intent** that mints, and this is a fresh intent. The
/// idempotent path is the API's, where the caller sends the id it already minted
/// (`migrate.local.md` §6.1).
pub fn add_custom_agent(
    store: &Store,
    label: &str,
    note: Option<&str>,
    protocol: Option<&str>,
) -> Result<CustomAgentVm, String> {
    let id = kiwano_api::ids::mint_agent_id(label);
    daemon::create_custom_agent(store, &id, label, note, protocol).map_err(|e| e.to_string())
}

/// Rename a user-defined agent — served by the daemon. Its id, route and key are
/// untouched: only the name its user reads changes.
pub fn update_custom_agent(
    store: &Store,
    id: &str,
    label: &str,
    note: Option<&str>,
    protocol: Option<&str>,
) -> Result<CustomAgentVm, String> {
    daemon::update_custom_agent(store, id, label, note, protocol).map_err(|e| e.to_string())
}

/// Delete a user-defined agent along with its route and its key — served by the
/// daemon. Its usage and request logs stay, so the Dashboard keeps accounting
/// for what ran.
pub fn remove_custom_agent(store: &Store, id: &str) -> Result<(), String> {
    daemon::remove_custom_agent(store, id).map_err(|e| e.to_string())
}

/// The protocols each built-in agent's own clients speak, in the vocabulary
/// `kiwanod::store::Protocol` uses (`"anthropic"`, `"openai"`, `"gemini"`).
///
/// Read off what this app already writes into each tool's config — that is the
/// wire format the tool then reads (`wire_api = "responses"` for Codex, `api:
/// "openai-completions"` for OpenClaw and WorkBuddy, `providers[].type =
/// "openai"` for Kimi, and so on through the rewriters), so it is evidence
/// rather than a catalogue of what each vendor also offers.
///
/// A parallel table keyed by the ids in [`AGENTS`] rather than a third field on
/// it: that registry is destructured as `(agent, label)` in a dozen places, and
/// its neighbours here (`ADDITIVE_AGENTS`, `REBUILDABLE_AGENTS`, `CLI_AGENTS`)
/// are the same shape for the same reason.
///
/// **A label.** Nothing routes, validates or filters by it: the gateway learns
/// an inbound's protocol from the path it was called on
/// (`gateway::protocol::classify_path`), and that is unchanged.
pub const AGENT_PROTOCOLS: [(&str, &[&str]); 26] = [
    ("claude", &["anthropic"]),
    ("codex", &["openai"]),
    ("gemini", &["gemini"]),
    ("grokbuild", &["openai"]),
    ("claude-desktop", &["anthropic"]),
    ("opencode", &["openai"]),
    ("openclaw", &["openai"]),
    ("hermes", &["openai"]),
    ("pi", &["openai"]),
    ("omp", &["openai"]),
    ("commandcode", &["openai"]),
    ("dsh", &["openai"]),
    ("hanaagent", &["openai"]),
    ("workbuddy", &["openai"]),
    ("codebuddy", &["openai"]),
    ("kimi", &["openai"]),
    ("qwen", &["openai"]),
    ("cline", &["openai"]),
    ("mimo", &["openai"]),
    ("mcode", &["openai"]),
    ("aider", &["openai"]),
    ("continue", &["openai"]),
    ("crush", &["openai"]),
    ("droid", &["openai"]),
    ("goose", &["openai"]),
    ("zcode", &["openai"]),
];

/// The protocols `agent` speaks, or an empty slice for an id nobody knows — a
/// user-defined agent is not in that table, and "we have no idea" is the honest
/// answer for one.
pub fn agent_protocols(agent: &str) -> &'static [&'static str] {
    AGENT_PROTOCOLS
        .iter()
        .find(|(id, _)| *id == agent)
        .map(|(_, protocols)| *protocols)
        .unwrap_or(&[])
}

/// Additive-mode agents: their native config keeps multiple providers
/// coexisting, so takeover writes a gateway-pointed provider entry and selects
/// it, instead of replacing an exclusive provider slot like the other five.
pub const ADDITIVE_AGENTS: [&str; 19] = [
    "opencode",
    "openclaw",
    "hermes",
    "pi",
    "omp",
    "commandcode",
    "dsh",
    "hanaagent",
    "workbuddy",
    "codebuddy",
    "kimi",
    "qwen",
    "mimo",
    "mcode",
    "continue",
    "crush",
    "droid",
    "goose",
    "zcode",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vm::dashboard::build_dashboard;
    use crate::vm::providers::build_provider_vms;
    use crate::vm::routes::add_agent_binding;
    use crate::vm::settings::build_settings_with_home;
    use crate::vm::takeover::set_agent_takeover;
    use crate::vm::takeover::StateHalf;
    use crate::vm::test_support::{live_home, no_vars, provider, store, usage_row};
    use crate::vm::Aux;
    use kiwanod::store::Billing;

    /// The takeover reference table in `docs/agent-takeover.md` is the user's
    /// answer to "what does a takeover write" — and prose is where
    /// documentation drifts. This pins the table to the registry: every
    /// built-in agent must have a row there, and every row must name a
    /// built-in. Adding an agent without adding its row fails here, in CI,
    /// instead of in a user's trust.
    #[test]
    fn the_docs_takeover_table_lists_every_builtin_agent() {
        let docs =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/agent-takeover.md");
        let text = std::fs::read_to_string(&docs).unwrap_or_else(|e| {
            panic!(
                "cannot read {} — the takeover table must ship with the registry: {e}",
                docs.display()
            )
        });
        let in_docs: Vec<&str> = text
            .lines()
            .filter(|l| l.starts_with("| `"))
            .filter_map(|l| l.split('|').nth(1))
            .map(|cell| cell.trim().trim_matches('`'))
            .collect();
        assert!(
            !in_docs.is_empty(),
            "the takeover table parsed to nothing — check its format"
        );
        for (id, _) in AGENTS {
            assert!(
                in_docs.contains(&id),
                "{id} is a built-in agent but has no row in docs/agent-takeover.md"
            );
        }
        for id in in_docs {
            assert!(
                is_builtin_agent(id),
                "docs/agent-takeover.md lists `{id}`, which the registry does not know"
            );
        }
    }

    /// A user-defined agent is a route and nothing else: a row, a key and a
    /// strategy. It needs no config file to route — the empty temp home below is
    /// the proof, since a built-in agent with the same binding reads as dormant
    /// there.
    #[test]
    fn a_custom_agent_routes_without_a_config_file() {
        let s = store();
        let aux = Aux::open_in_memory().unwrap();
        s.insert_provider(&provider("p1", "Alpha", Billing::Metered))
            .unwrap();
        let a = add_custom_agent(&s, "Long Tasks", Some("night batch"), Some("gemini")).unwrap();

        assert!(a.id.starts_with("long-tasks-"), "{}", a.id);
        assert_eq!(
            a.protocol.as_deref(),
            Some("gemini"),
            "the protocol it was created with travels back"
        );
        assert!(!is_builtin_agent(&a.id));
        let key = a.placeholder_key.clone().expect("a key is minted with it");
        assert!(key.starts_with(&format!("kw-ag-{}", a.id)), "{key}");

        add_agent_binding(&s, &a.id, "p1").unwrap();
        add_agent_binding(&s, "claude", "p1").unwrap(); // the control: dormant here
        let home = live_home(&[]); // nothing taken over on this machine
        let vms = build_provider_vms(&s, &aux, home.path(), &no_vars()).unwrap();
        let vm = &vms[0];
        assert_eq!(vm.agents, [a.id.as_str()], "the custom agent is bound");
        assert_eq!(
            vm.serving_agents,
            [a.id.as_str()],
            "…and it serves: it has no config for the evidence to be missing from"
        );
        assert!(vm.is_current);

        // The UI gets it, with the key it is configured by, and the takeover
        // list is unmoved: there is no config here to take over.
        let settings = build_settings_with_home(&s, &aux, home.path(), &no_vars()).unwrap();
        assert_eq!(settings.custom_agents.len(), 1);
        assert_eq!(settings.custom_agents[0].label, "Long Tasks");
        assert_eq!(
            settings.custom_agents[0].protocol.as_deref(),
            Some("gemini")
        );
        assert_eq!(
            settings.custom_agents[0].placeholder_key.as_deref(),
            Some(key.as_str())
        );
        assert_eq!(
            settings.custom_agents[0].note.as_deref(),
            Some("night batch")
        );
        assert!(settings.takeovers.iter().all(|t| !t.enabled));
        assert!(
            set_agent_takeover(
                &s,
                &aux,
                &a.id,
                true,
                8317,
                home.path(),
                &no_vars(),
                StateHalf::InProcess,
            )
            .is_err(),
            "takeover is for agents with a config"
        );
    }

    /// Deleting a custom agent takes its route and its key with it and leaves
    /// the usage history — the same line provider deletion draws. Its traffic
    /// stays on the dashboard, under the id, because the rows that recorded it
    /// are not the route that was deleted.
    #[test]
    fn removing_a_custom_agent_clears_its_route_and_keeps_its_history() {
        let s = store();
        let aux = Aux::open_in_memory().unwrap();
        s.insert_provider(&provider("p1", "Alpha", Billing::Metered))
            .unwrap();
        let a = add_custom_agent(&s, "Long Tasks", None, None).unwrap();
        add_agent_binding(&s, &a.id, "p1").unwrap();
        let mut row = usage_row("p1");
        row.agent = a.id.clone();
        row.cost = Some(2.0);
        row.cost_currency = Some("CNY".into());
        s.record_usage(&row).unwrap();

        remove_custom_agent(&s, &a.id).unwrap();

        assert!(s.bindings_for_agent(&a.id).unwrap().is_empty());
        assert!(s.get_strategy(&a.id).unwrap().is_none());
        assert!(s
            .list_placeholder_keys()
            .unwrap()
            .iter()
            .all(|k| k.agent != a.id));
        assert!(s.get_custom_agent(&a.id).unwrap().is_none());
        // The provider is the user's row and stays; the usage is history.
        assert!(s.get_provider("p1").unwrap().is_some());
        assert_eq!(s.usage_totals(Some(&a.id), None, None).unwrap().requests, 1);

        // The dashboard still accounts for that traffic, labelled by its id —
        // the agent's name is gone, its requests are not.
        let d = build_dashboard(&s, &aux, "7d", None, None).unwrap();
        let row = d
            .by_agent
            .iter()
            .find(|r| r.agent == a.id)
            .expect("history outlives the route");
        assert_eq!(row.label, a.id);
        assert!(d.filter_agents.iter().any(|f| f.id == a.id));

        // And it is gone as an agent: a second removal has nothing to remove.
        assert!(remove_custom_agent(&s, &a.id).is_err());
    }

    /// A rename moves the label — and the note — and nothing else. The id is
    /// what bindings, keys and usage rows point at, so a rename that touched it
    /// would orphan the agent's route and its history.
    #[test]
    fn renaming_a_custom_agent_keeps_its_id_route_and_history() {
        let s = store();
        let aux = Aux::open_in_memory().unwrap();
        s.insert_provider(&provider("p1", "Alpha", Billing::Metered))
            .unwrap();
        let a = add_custom_agent(&s, "Long Tasks", Some("at night"), None).unwrap();
        add_agent_binding(&s, &a.id, "p1").unwrap();
        let mut row = usage_row("p1");
        row.agent = a.id.clone();
        s.record_usage(&row).unwrap();

        let renamed = update_custom_agent(&s, &a.id, "Nightly batch", Some("moved"), None).unwrap();

        assert_eq!(renamed.id, a.id, "the id is what everything points at");
        assert_eq!(renamed.label, "Nightly batch");
        assert_eq!(renamed.note.as_deref(), Some("moved"));
        assert_eq!(
            renamed.placeholder_key, a.placeholder_key,
            "the key belongs to the agent, not to the name its user gave it"
        );
        // The route and the history are still this agent's.
        assert_eq!(s.bindings_for_agent(&a.id).unwrap().len(), 1);
        assert!(s.get_strategy(&a.id).unwrap().is_some());
        assert_eq!(s.usage_totals(Some(&a.id), None, None).unwrap().requests, 1);
        // And the new name is what the dashboard reads for it now.
        let d = build_dashboard(&s, &aux, "7d", None, None).unwrap();
        assert_eq!(
            d.by_agent
                .iter()
                .find(|r| r.agent == a.id)
                .expect("still accounted for")
                .label,
            "Nightly batch"
        );
        // A blank note clears it rather than storing whitespace.
        let cleared = update_custom_agent(&s, &a.id, "Nightly batch", Some("  "), None).unwrap();
        assert_eq!(cleared.note, None);

        // Refusals: a blank name, and an id no agent has — and neither changes
        // what is on file.
        assert!(update_custom_agent(&s, &a.id, "   ", None, None).is_err());
        assert!(update_custom_agent(&s, "no-such-agent", "X", None, None).is_err());
        let stored = s.get_custom_agent(&a.id).unwrap().expect("still there");
        assert_eq!(stored.label, "Nightly batch");
        assert_eq!(stored.id, a.id);
    }

    /// The model a latency ping uses: the provider's own default, else the model
    /// its catalog entry prices — the one model we know the vendor serves. With
    /// neither, there is nothing to ping with, and the caller is told that
    /// rather than handed a model id this code invented.
    #[test]
    fn the_latency_ping_uses_the_providers_own_model_or_its_entrys() {
        let s = store();
        let catalog = r#"{"total":1,"entries":[
            {"id":"deepseek","name":"DeepSeek","tag":"official","rating":4.8,
             "billing":"payg","currency":"USD",
             "endpoints":[{"protocol":"openai","endpoint":"https://api.deepseek.com"}],
             "price_ref":{"model_id":"deepseek-chat","display_name":"DeepSeek Chat",
                          "input":"1","output":"2","currency":"USD"}}]}"#;
        // The cache is the daemon's table (`store::hub`), so it goes in the
        // store — writing it into an aux would leave this reading an empty one.
        s.save_hub_cache(catalog, "2026-09-07T00:00:00Z").unwrap();

        let mut p = provider("p1", "DeepSeek", Billing::Metered);
        p.catalog_id = Some("deepseek".into());
        s.insert_provider(&p).unwrap();
        assert_eq!(
            kiwanod::api::probe::prompt_test_model(&s, &p).as_deref(),
            Some("deepseek-chat")
        );

        // A default of its own wins: it is the model the user routes with.
        let mut with_default = p.clone();
        with_default.model_default = Some("deepseek-v4-flash".into());
        assert_eq!(
            kiwanod::api::probe::prompt_test_model(&s, &with_default).as_deref(),
            Some("deepseek-v4-flash")
        );

        let orphan = provider("p2", "No Catalog", Billing::Metered);
        assert_eq!(kiwanod::api::probe::prompt_test_model(&s, &orphan), None);
    }

    /// The id is derived from the name, is unique per agent even when the name
    /// repeats, and has a word for it even when the name has no ASCII in it.
    /// The two tables that describe a built-in agent have to agree about which
    /// agents exist: a registry entry with no protocols would render as "speaks
    /// nothing", and a protocol row for an id nobody knows is dead weight. Five
    /// parallel tables already drift silently in this codebase (the frontend's
    /// `AGENTS`, `AGENT_ICON` and `SEGMENTS`, plus `CLI_AGENTS` here) — this one
    /// is pinned to its neighbour.
    #[test]
    fn every_built_in_agent_has_at_least_one_protocol() {
        for (agent, label) in AGENTS {
            let protocols = agent_protocols(agent);
            assert!(
                !protocols.is_empty(),
                "{agent} ({label}) is in the registry with no protocol"
            );
            for p in protocols {
                assert!(
                    kiwanod::store::Protocol::parse_str(p).is_some(),
                    "{agent} names a protocol nobody knows: {p}"
                );
            }
        }
        assert_eq!(
            AGENT_PROTOCOLS.len(),
            AGENTS.len(),
            "the two tables describe different numbers of agents"
        );
        // And an id that is not a built-in gets nothing, rather than a guess.
        assert!(agent_protocols("long-tasks-3f9a").is_empty());
    }

    /// The protocol is a word three readers recognise, so a typo is refused
    /// rather than stored — and clearing it is how a user says they would
    /// rather not say.
    #[test]
    fn a_protocol_that_is_not_a_protocol_is_refused() {
        let s = store();
        let err = match add_custom_agent(&s, "Typo", None, Some("opemai")) {
            Ok(vm) => panic!("a typo was accepted: {}", vm.id),
            Err(e) => e,
        };
        assert!(err.contains("unknown protocol"), "{err}");

        let a = add_custom_agent(&s, "Fine", None, Some("  anthropic ")).unwrap();
        assert_eq!(a.protocol.as_deref(), Some("anthropic"), "trimmed");

        let cleared = update_custom_agent(&s, &a.id, "Fine", None, Some("")).unwrap();
        assert_eq!(cleared.protocol, None, "an empty choice is no choice");
    }

    #[test]
    fn custom_agent_ids_are_derived_and_unique() {
        let s = store();
        let first = add_custom_agent(&s, "Long Tasks", None, None).unwrap();
        let second = add_custom_agent(&s, "Long Tasks", None, None).unwrap();
        assert!(first.id.starts_with("long-tasks-"));
        assert!(second.id.starts_with("long-tasks-"));
        assert_ne!(first.id, second.id, "same name, two agents");
        assert!(!is_builtin_agent(&first.id));

        // A name with nothing slug-able in it still gets a usable id — and not
        // `slug`'s own fallback word, which belongs to providers.
        let cjk = add_custom_agent(&s, "长任务批处理", None, None).unwrap();
        assert!(cjk.id.starts_with("custom-"), "{}", cjk.id);

        // A name is required; whitespace is not one.
        assert!(add_custom_agent(&s, "   ", None, None).is_err());
        // …and a blank note is the same as no note.
        let blank = add_custom_agent(&s, "Bare", Some("  "), None).unwrap();
        assert_eq!(blank.note, None);
    }

    /// What a latency test's three outcomes become on the row. The middle one is
    /// the whole point of the `error` column: a refusal is the vendor answering,
    /// which reachability alone cannot tell apart from silence.
    #[test]
    fn a_test_verdict_separates_a_refusal_from_silence() {
        use crate::sidecar::PromptProbe;

        // Answered, accepted.
        let ok = Ok(PromptProbe {
            latency_ms: 218,
            status: 200,
            error: None,
        });
        assert_eq!(
            kiwanod::api::probe::test_verdict(&ok),
            ("reachable", 218, None)
        );

        // Answered, refused: reachable, and the reason travels with it.
        let refused = Ok(PromptProbe {
            latency_ms: 60,
            status: 401,
            error: Some("invalid API key".into()),
        });
        let (status, ms, error) = kiwanod::api::probe::test_verdict(&refused);
        assert_eq!(
            status, "reachable",
            "the vendor answered — that is the fact"
        );
        assert_eq!(ms, 60);
        assert_eq!(error.as_deref(), Some("invalid API key"));

        // Nobody answered.
        let dead = Err("connection failed: dns error".to_string());
        let (status, ms, error) = kiwanod::api::probe::test_verdict(&dead);
        assert_eq!(status, "down");
        assert_eq!(ms, 0);
        assert_eq!(error.as_deref(), Some("connection failed: dns error"));
    }
}
