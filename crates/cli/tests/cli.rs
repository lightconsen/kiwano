//! End-to-end tests for the command tree.
//!
//! Driven through `run_with` with a temporary database, asserting on the exit
//! code, stdout and stderr separately — the split between the last two is part
//! of the contract, not an implementation detail: stdout is the payload, so a
//! `--json` consumer can pipe it without a diagnostic landing in the middle.
//!
//! Nothing here reaches a real gateway. Commands that would reload discover no
//! admin plane beside the temp database and say so on stderr.

use std::path::{Path, PathBuf};

use kiwanod::store::Store;

/// Run the CLI against `db`, returning (exit code, stdout, stderr).
fn run(db: &Path, args: &[&str]) -> (i32, String, String) {
    let mut argv = vec!["--db".to_string(), db.display().to_string()];
    argv.extend(args.iter().map(|s| s.to_string()));
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = kiwano_cli::run_with(&argv, &mut out, &mut err);
    (
        code,
        String::from_utf8_lossy(&out).into_owned(),
        String::from_utf8_lossy(&err).into_owned(),
    )
}

fn temp_db() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("kiwano.db");
    (dir, db)
}

/// Add a provider and return its generated id.
fn add_provider(db: &Path, name: &str, bind: &[&str]) -> String {
    let mut args = vec!["--json", "providers", "add", "--name", name, "--endpoint"];
    let endpoint = format!("https://{name}.example.com");
    args.push(&endpoint);
    args.push("--key");
    args.push("sk-test");
    for agent in bind {
        args.push("--bind");
        args.push(agent);
    }
    let (code, out, err) = run(db, &args);
    assert_eq!(code, 0, "add failed: {err}");
    let v: serde_json::Value = serde_json::from_str(&out).expect("add emits JSON under --json");
    v["id"].as_str().expect("id").to_string()
}

// ── globals ─────────────────────────────────────────────────────────────────

/// Globals are position-independent: the hand-rolled parser accepted them
/// anywhere and scripts are written both ways, so clap has to agree.
#[test]
fn globals_are_position_independent() {
    let (_dir, db) = temp_db();
    let (a, out_a, _) = run(&db, &["providers", "list", "--json"]);
    let (b, out_b, _) = run(&db, &["--json", "providers", "list"]);
    assert_eq!(a, 0);
    assert_eq!(b, 0);
    assert_eq!(out_a, out_b);
}

#[test]
fn unknown_command_is_a_usage_error() {
    let (_dir, db) = temp_db();
    let (code, out, err) = run(&db, &["frobnicate"]);
    assert_eq!(code, 2, "clap's usage code, and the documented one");
    assert!(out.is_empty(), "nothing on stdout: {out}");
    assert!(err.contains("frobnicate"), "{err}");
}

// ── providers ───────────────────────────────────────────────────────────────

#[test]
fn providers_add_then_list_roundtrips() {
    let (_dir, db) = temp_db();
    let id = add_provider(&db, "alpha", &[]);

    let (code, out, _) = run(&db, &["--json", "providers", "list"]);
    assert_eq!(code, 0);
    let list: Vec<serde_json::Value> = serde_json::from_str(&out).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["id"], id);
    assert_eq!(list[0]["name"], "alpha");
    // The view model carries the *display* endpoint — scheme stripped, as the
    // app's provider rows show it. The stored value is the full URL.
    assert_eq!(list[0]["endpoint"], "alpha.example.com");
    assert_eq!(list[0]["protocol"], "openai");
    // The app's vocabulary, not the CLI's old one.
    assert_eq!(list[0]["billing"], "payg");

    // …and it is really in the store, not just in the rendering.
    let store = Store::open(&db).unwrap();
    let stored = store.list_providers().unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].base_url, "https://alpha.example.com");
}

#[test]
fn providers_list_filters_by_agent() {
    let (_dir, db) = temp_db();
    add_provider(&db, "for-claude", &["claude"]);
    add_provider(&db, "unbound", &[]);

    let (_, out, _) = run(&db, &["--json", "providers", "list", "--agent", "claude"]);
    let list: Vec<serde_json::Value> = serde_json::from_str(&out).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["name"], "for-claude");
}

/// Billing words from the old CLI keep working, and both spellings land in the
/// store as the same row.
#[test]
fn billing_accepts_both_vocabularies() {
    let (_dir, db) = temp_db();
    let (code, out, err) = run(
        &db,
        &[
            "--json",
            "providers",
            "add",
            "--name",
            "legacy",
            "--endpoint",
            "https://legacy.example.com",
            "--billing",
            "metered",
        ],
    );
    assert_eq!(code, 0, "{err}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["billing"], "payg");

    let (code, _, err) = run(
        &db,
        &[
            "providers",
            "add",
            "--name",
            "nope",
            "--endpoint",
            "https://nope.example.com",
            "--billing",
            "per-token",
        ],
    );
    assert_eq!(code, 2, "an unknown billing tag is a usage error");
    assert!(err.contains("per-token"), "{err}");
}

/// The GUI drops the limit columns for plan providers; this refuses the flags
/// instead of accepting them and quietly writing nothing.
#[test]
fn plan_providers_reject_limit_flags() {
    let (_dir, db) = temp_db();
    let (code, _, err) = run(
        &db,
        &[
            "providers",
            "add",
            "--name",
            "planprov",
            "--endpoint",
            "https://plan.example.com",
            "--billing",
            "plan",
            "--limit",
            "50",
        ],
    );
    assert_eq!(code, 2);
    assert!(err.contains("plan providers"), "{err}");
}

#[test]
fn providers_use_makes_primary_and_demotes_previous() {
    let (_dir, db) = temp_db();
    let first = add_provider(&db, "first", &["claude"]);
    let second = add_provider(&db, "second", &[]);

    let (code, _, err) = run(&db, &["providers", "use", &second, "--agent", "claude"]);
    assert_eq!(code, 0, "{err}");

    let store = Store::open(&db).unwrap();
    assert_eq!(
        store.primary_provider_id("claude").unwrap().as_deref(),
        Some(second.as_str())
    );
    let bindings = store.bindings_for_agent("claude").unwrap();
    assert_eq!(bindings.len(), 2, "the old primary stays as a candidate");
    assert_eq!(bindings[0].provider_id, second);
    assert_eq!(bindings[0].priority, 0);
    assert_eq!(bindings[1].provider_id, first);
    // Reindexed, not left sharing priority 0 with the new primary.
    assert_eq!(bindings[1].priority, 1);
}

#[test]
fn providers_use_rejects_unknown_provider() {
    let (_dir, db) = temp_db();
    let (code, _, err) = run(&db, &["providers", "use", "nope", "--agent", "claude"]);
    assert_eq!(code, 3, "a runtime error, not a usage error");
    assert!(err.contains("provider not found"), "{err}");
}

#[test]
fn providers_remove_promotes_the_next_candidate() {
    let (_dir, db) = temp_db();
    let first = add_provider(&db, "first", &["claude"]);
    let second = add_provider(&db, "second", &[]);
    run(&db, &["providers", "use", &second, "--agent", "claude"]);

    let (code, _, err) = run(&db, &["providers", "remove", &second]);
    assert_eq!(code, 0, "{err}");

    let store = Store::open(&db).unwrap();
    assert_eq!(
        store.primary_provider_id("claude").unwrap().as_deref(),
        Some(first.as_str()),
        "removing the primary promotes what was behind it"
    );
}

// ── forwarding options (advanced / plan limits / plan query) ────────────────

#[test]
fn providers_add_stores_the_forwarding_options() {
    let (_dir, db) = temp_db();
    let (code, out, err) = run(
        &db,
        &[
            "--json",
            "providers",
            "add",
            "--name",
            "azure",
            "--endpoint",
            "https://x.openai.azure.com",
            "--protocol",
            "openai",
            "--timeout",
            "120",
            "--retries",
            "2",
            "--header",
            "api-key: az-secret",
            // A header value may itself contain a colon — the split is on the
            // first one only, which matters for the many that do.
            "--header",
            "X-Trace: a:b:c",
            "--endpoint-extra",
            "anthropic=https://x.anthropic.azure.com",
        ],
    );
    assert_eq!(code, 0, "{err}");
    let _: serde_json::Value = serde_json::from_str(&out).unwrap();

    let store = Store::open(&db).unwrap();
    let row = store.list_providers().unwrap().into_iter().next().unwrap();
    assert_eq!(row.timeout_secs, Some(120));
    assert_eq!(row.retries, Some(2));
    let headers = row.headers.expect("headers stored");
    assert!(headers.contains("az-secret"), "{headers}");
    assert!(headers.contains("a:b:c"), "value kept whole: {headers}");
    assert_eq!(row.endpoints.len(), 1);
    assert_eq!(row.endpoints[0].base_url, "https://x.anthropic.azure.com");
}

/// `advanced` is an authoritative snapshot in `vm`: setting one field rewrites
/// all three columns. An edit that names only one must therefore carry the rest
/// over from the stored row, or it silently clears them.
#[test]
fn providers_edit_preserves_the_forwarding_options_it_was_not_given() {
    let (_dir, db) = temp_db();
    let id = add_provider(&db, "azure", &[]);
    run(
        &db,
        &[
            "providers",
            "edit",
            &id,
            "--timeout",
            "120",
            "--header",
            "api-key: az-secret",
            "--endpoint-extra",
            "anthropic=https://alt.example.com",
        ],
    );
    let (code, _, err) = run(&db, &["providers", "edit", &id, "--name", "renamed"]);
    assert_eq!(code, 0, "{err}");

    let store = Store::open(&db).unwrap();
    let row = store.get_provider(&id).unwrap().unwrap();
    assert_eq!(row.name, "renamed");
    assert_eq!(row.timeout_secs, Some(120), "timeout survived a rename");
    assert!(row.headers.is_some(), "headers survived a rename");
    assert_eq!(row.endpoints.len(), 1, "endpoints survived a rename");

    // Changing one field still leaves the other two alone.
    let (code, _, err) = run(&db, &["providers", "edit", &id, "--retries", "3"]);
    assert_eq!(code, 0, "{err}");
    let store = Store::open(&db).unwrap();
    let row = store.get_provider(&id).unwrap().unwrap();
    assert_eq!(row.retries, Some(3));
    assert_eq!(row.timeout_secs, Some(120));

    // …and clearing them is explicit, not a side effect of an unrelated edit.
    let (code, _, err) = run(&db, &["providers", "edit", &id, "--no-headers"]);
    assert_eq!(code, 0, "{err}");
    let store = Store::open(&db).unwrap();
    let row = store.get_provider(&id).unwrap().unwrap();
    assert_eq!(row.headers, None);
    assert_eq!(row.timeout_secs, Some(120), "only the headers were cleared");
}

#[test]
fn forwarding_flags_reject_malformed_values() {
    let (_dir, db) = temp_db();
    for (args, needle) in [
        (vec!["--header", "no-colon-here"], "Name: value"),
        (vec!["--header", ": empty-name"], "empty name"),
        (vec!["--endpoint-extra", "not-a-pair"], "PROTO=URL"),
        (
            vec!["--endpoint-extra", "grpc=https://x.example.com"],
            "grpc",
        ),
        (vec!["--endpoint-extra", "openai="], "empty URL"),
    ] {
        let mut argv = vec![
            "providers",
            "add",
            "--name",
            "x",
            "--endpoint",
            "https://x.example.com",
        ];
        argv.extend(args.iter().copied());
        let (code, _, err) = run(&db, &argv);
        assert_eq!(code, 2, "should be a usage error: {argv:?} — {err}");
        assert!(err.contains(needle), "{argv:?} → {err}");
    }
}

#[test]
fn plan_limits_require_plan_billing() {
    let (_dir, db) = temp_db();
    let (code, _, err) = run(
        &db,
        &[
            "providers",
            "add",
            "--name",
            "x",
            "--endpoint",
            "https://x.example.com",
            "--plan-limit-5h",
            "20",
        ],
    );
    assert_eq!(code, 2, "payg has no plan windows to limit");
    assert!(err.contains("plan providers"), "{err}");

    let (code, _, err) = run(
        &db,
        &[
            "providers",
            "add",
            "--name",
            "p",
            "--endpoint",
            "https://p.example.com",
            "--billing",
            "plan",
            "--plan-limit-5h",
            "20",
            "--plan-limit-weekly",
            "60",
        ],
    );
    assert_eq!(code, 0, "{err}");
    let store = Store::open(&db).unwrap();
    let row = store.list_providers().unwrap().into_iter().next().unwrap();
    let limits = row.plan_limits.expect("stored");
    assert!(limits.contains("20"), "{limits}");
    assert!(limits.contains("60"), "{limits}");
}

/// The plan query is what `providers quota` reads. Without a way to set it, that
/// command could only ever work on providers the desktop app had configured.
#[test]
fn plan_query_round_trips_and_clears() {
    let (_dir, db) = temp_db();
    let (code, out, err) = run(
        &db,
        &[
            "--json",
            "providers",
            "add",
            "--name",
            "kimi",
            "--endpoint",
            "https://api.moonshot.cn/anthropic",
            "--billing",
            "plan",
            "--plan-query",
            r#"{"template":"kimi","fields":{"api_key":"sk-x"}}"#,
        ],
    );
    assert_eq!(code, 0, "{err}");
    let created: serde_json::Value = serde_json::from_str(&out).unwrap();
    let id = created["id"].as_str().unwrap().to_string();

    let store = Store::open(&db).unwrap();
    let row = store.get_provider(&id).unwrap().unwrap();
    assert!(row.plan_query.as_deref().unwrap().contains("kimi"));

    // Malformed JSON, and a non-object, are both refused.
    let (code, _, err) = run(&db, &["providers", "edit", &id, "--plan-query", "not json"]);
    assert_eq!(code, 2);
    assert!(err.contains("valid JSON"), "{err}");
    let (code, _, err) = run(&db, &["providers", "edit", &id, "--plan-query", "[]"]);
    assert_eq!(code, 2);
    assert!(err.contains("JSON object"), "{err}");

    // Clearing is explicit — an absent --plan-query means "keep".
    let (code, _, err) = run(&db, &["providers", "edit", &id, "--name", "kimi2"]);
    assert_eq!(code, 0, "{err}");
    let store = Store::open(&db).unwrap();
    assert!(store
        .get_provider(&id)
        .unwrap()
        .unwrap()
        .plan_query
        .is_some());

    let (code, _, err) = run(&db, &["providers", "edit", &id, "--clear-plan-query"]);
    assert_eq!(code, 0, "{err}");
    let store = Store::open(&db).unwrap();
    assert_eq!(store.get_provider(&id).unwrap().unwrap().plan_query, None);

    // And with none configured, quota says so rather than inventing an answer.
    let (code, _, err) = run(&db, &["providers", "quota", &id]);
    assert_eq!(code, 3);
    assert!(err.to_lowercase().contains("plan query"), "{err}");
}

// ── keys ────────────────────────────────────────────────────────────────────

#[test]
fn keys_add_list_remove_roundtrips() {
    let (_dir, db) = temp_db();
    let provider = add_provider(&db, "rotating", &[]);

    let (code, out, err) = run(
        &db,
        &[
            "--json",
            "keys",
            "add",
            &provider,
            "--key",
            "sk-rotating-key-0001",
            "--label",
            "spare",
        ],
    );
    assert_eq!(code, 0, "{err}");
    let added: serde_json::Value = serde_json::from_str(&out).unwrap();
    let key_id = added["id"].as_i64().expect("id");
    // Never the plaintext, even locally: head and tail are kept so two entries
    // can be told apart, and the middle — the part that matters — is not.
    let masked = added["masked"].as_str().unwrap();
    assert!(
        !masked.contains("rotating"),
        "the key body must not survive masking, got {masked}"
    );
    assert_ne!(masked, "sk-rotating-key-0001");
    assert!(masked.starts_with("sk-rot"), "head kept, got {masked}");

    let (_, out, _) = run(&db, &["--json", "keys", "list", &provider]);
    let keys: Vec<serde_json::Value> = serde_json::from_str(&out).unwrap();
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0]["label"], "spare");

    let (code, _, err) = run(&db, &["keys", "remove", &key_id.to_string()]);
    assert_eq!(code, 0, "{err}");
    let (_, out, _) = run(&db, &["--json", "keys", "list", &provider]);
    let keys: Vec<serde_json::Value> = serde_json::from_str(&out).unwrap();
    assert!(keys.is_empty());
}

// ── usage / status ──────────────────────────────────────────────────────────

#[test]
fn usage_reports_an_empty_window_without_failing() {
    let (_dir, db) = temp_db();
    let (code, out, err) = run(&db, &["usage"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("requests 0"), "{out}");
    assert!(out.contains("(no usage in window)"), "{out}");
}

#[test]
fn usage_rejects_a_non_positive_window() {
    let (_dir, db) = temp_db();
    let (code, _, err) = run(&db, &["usage", "--days", "0"]);
    assert_eq!(code, 2);
    assert!(err.contains("--days"), "{err}");
}

/// Gateway down is an answer, not a failure — exit 1, and the store is still
/// reported so a headless operator can see what is configured.
#[test]
fn status_without_a_gateway_exits_1_and_reports_the_store() {
    let (_dir, db) = temp_db();
    add_provider(&db, "alpha", &[]);
    let (code, out, _) = run(&db, &["status"]);
    assert_eq!(code, 1);
    assert!(out.contains("gateway: not running"), "{out}");
    assert!(out.contains("providers 1"), "{out}");
    // Today's totals, which the app's status bar shows. They come from the
    // *usage* table — what was billable — not from the request log, so seeding
    // a log row would not move them.
    {
        let store = Store::open(&db).unwrap();
        store
            .record_usage(&kiwanod::store::UsageRecord {
                ts: kiwanod::store::now_rfc3339(),
                agent: "claude".into(),
                provider_id: "alpha".into(),
                model: None,
                input_tokens: 100,
                output_tokens: 20,
                cache_read_tokens: 0,
                cache_creation_tokens: 0,
                latency_ms: None,
                status: "ok".into(),
                cost: None,
                cost_currency: None,
            })
            .unwrap();
    }
    let (_, out, _) = run(&db, &["status"]);
    assert!(out.contains("today: 1 requests"), "{out}");
    assert!(out.contains("120 tokens"), "{out}");
}

/// `status` is the first thing anyone runs on a machine with no database, so it
/// must not create one — opening the store does that as a side effect.
#[test]
fn status_does_not_create_a_database() {
    let (dir, db) = temp_db();
    assert!(!db.exists());
    let (code, _, _) = run(&db, &["status"]);
    assert_eq!(code, 1);
    assert!(
        !db.exists(),
        "asking whether a gateway is running should not conjure the store"
    );
    let _ = dir;
}

/// The id is the whole point of `edit`: bindings, rotating keys and usage rows
/// all reference it, so remove-and-add is not the same operation.
#[test]
fn providers_edit_keeps_the_id_and_the_fields_it_was_not_given() {
    let (_dir, db) = temp_db();
    let id = add_provider(&db, "original", &["claude"]);

    let (code, out, err) = run(
        &db,
        &[
            "--json",
            "providers",
            "edit",
            &id,
            "--name",
            "renamed",
            "--endpoint",
            "https://moved.example.com",
        ],
    );
    assert_eq!(code, 0, "{err}");
    let updated: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(updated["id"], id, "the id must survive the edit");

    // Fields the flags did not mention are carried over, not blanked — that is
    // the failure mode `update_provider`'s authoritative semantics invite.
    let store = Store::open(&db).unwrap();
    let row = store.get_provider(&id).unwrap().unwrap();
    assert_eq!(row.name, "renamed");
    assert_eq!(row.base_url, "https://moved.example.com");
    assert_eq!(row.api_key.as_deref(), Some("sk-test"), "the key is kept");
    assert_eq!(row.protocol.as_str(), "openai", "the protocol is kept");
    assert_eq!(
        store.primary_provider_id("claude").unwrap().as_deref(),
        Some(id.as_str()),
        "the binding is kept"
    );
}

#[test]
fn providers_edit_can_unbind_from_every_agent() {
    let (_dir, db) = temp_db();
    let id = add_provider(&db, "bound", &["claude"]);
    let (code, _, err) = run(&db, &["providers", "edit", &id, "--no-bind"]);
    assert_eq!(code, 0, "{err}");
    let store = Store::open(&db).unwrap();
    assert!(store.bindings_for_agent("claude").unwrap().is_empty());
}

/// A provider with no plan query has no quota to report, and saying so beats a
/// fabricated 0%.
#[test]
fn providers_quota_reports_a_missing_plan_query() {
    let (_dir, db) = temp_db();
    let id = add_provider(&db, "plain", &[]);
    let (code, out, err) = run(&db, &["providers", "quota", &id]);
    assert_eq!(code, 3);
    assert!(err.to_lowercase().contains("plan query"), "{err}");
    assert!(out.is_empty(), "no payload on stdout for a failure: {out}");
}

#[test]
fn providers_enable_refuses_an_unbound_provider() {
    let (_dir, db) = temp_db();
    let id = add_provider(&db, "loose", &[]);
    let (code, _, err) = run(&db, &["providers", "enable", &id]);
    assert_eq!(code, 3);
    assert!(err.contains("not bound"), "{err}");
}

// ── routes ──────────────────────────────────────────────────────────────────

#[test]
fn strategy_roundrobin_seeds_balanced_weights() {
    let (_dir, db) = temp_db();
    add_provider(&db, "one", &["claude"]);
    add_provider(&db, "two", &["claude"]);

    let (code, _, err) = run(&db, &["routes", "strategy", "claude", "roundrobin"]);
    assert_eq!(code, 0, "{err}");

    let store = Store::open(&db).unwrap();
    let weights: Vec<i64> = store
        .bindings_for_agent("claude")
        .unwrap()
        .iter()
        .map(|b| b.weight)
        .collect();
    assert_eq!(weights, vec![50, 50], "an even split, not every row at 1");
}

/// The quota payload is validated by the engine's own parser, so a config this
/// writes cannot be one the engine would silently reinterpret.
#[test]
fn quota_strategy_requires_a_limit_and_a_known_unit() {
    let (_dir, db) = temp_db();
    add_provider(&db, "one", &["claude"]);

    let (code, _, err) = run(&db, &["routes", "strategy", "claude", "quota"]);
    assert_eq!(code, 2);
    assert!(err.contains("--limit"), "{err}");

    let (code, _, err) = run(
        &db,
        &[
            "routes", "strategy", "claude", "quota", "--limit", "10", "--unit", "cost",
        ],
    );
    assert_eq!(code, 2);
    assert!(err.contains("cost"), "{err}");

    let (code, _, err) = run(
        &db,
        &[
            "routes", "strategy", "claude", "quota", "--limit", "5000", "--unit", "tokens",
        ],
    );
    assert_eq!(code, 0, "{err}");
    let store = Store::open(&db).unwrap();
    let config = store
        .get_strategy("claude")
        .unwrap()
        .unwrap()
        .config
        .unwrap();
    assert!(config.contains("tokens"), "{config}");

    // A limit on a strategy that ignores one is a mistake, not a no-op.
    let (code, _, err) = run(
        &db,
        &["routes", "strategy", "claude", "failover", "--limit", "10"],
    );
    assert_eq!(code, 2);
    assert!(err.contains("quota"), "{err}");
}

#[test]
fn routes_binding_add_set_and_remove() {
    let (_dir, db) = temp_db();
    let first = add_provider(&db, "first", &["claude"]);
    let second = add_provider(&db, "second", &[]);

    let (code, _, err) = run(&db, &["routes", "binding", "add", "claude", &second]);
    assert_eq!(code, 0, "{err}");
    let store = Store::open(&db).unwrap();
    assert_eq!(store.bindings_for_agent("claude").unwrap().len(), 2);

    let (code, _, err) = run(
        &db,
        &[
            "routes",
            "binding",
            "set",
            "claude",
            &second,
            "--window",
            "09:00-17:00",
        ],
    );
    assert_eq!(code, 0, "{err}");
    let store = Store::open(&db).unwrap();
    let b = store
        .bindings_for_agent("claude")
        .unwrap()
        .into_iter()
        .find(|b| b.provider_id == second)
        .unwrap();
    assert_eq!(b.win_start.as_deref(), Some("09:00"));
    assert_eq!(b.win_end.as_deref(), Some("17:00"));

    let (code, _, err) = run(
        &db,
        &["routes", "binding", "set", "claude", &second, "--no-window"],
    );
    assert_eq!(code, 0, "{err}");
    let store = Store::open(&db).unwrap();
    let b = store
        .bindings_for_agent("claude")
        .unwrap()
        .into_iter()
        .find(|b| b.provider_id == second)
        .unwrap();
    assert_eq!(b.win_start, None, "both bounds clear together");

    // A malformed window is a usage error, not a silently ignored flag.
    let (code, _, err) = run(
        &db,
        &[
            "routes", "binding", "set", "claude", &second, "--window", "9am-5pm",
        ],
    );
    assert_eq!(code, 2);
    assert!(err.contains("HH:MM"), "{err}");

    let (code, _, err) = run(&db, &["routes", "binding", "remove", "claude", &second]);
    assert_eq!(code, 0, "{err}");
    let store = Store::open(&db).unwrap();
    let remaining = store.bindings_for_agent("claude").unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].provider_id, first);
}

#[test]
fn routes_reorder_rewrites_priorities() {
    let (_dir, db) = temp_db();
    let first = add_provider(&db, "first", &["claude"]);
    let second = add_provider(&db, "second", &["claude"]);
    assert_eq!(
        Store::open(&db)
            .unwrap()
            .primary_provider_id("claude")
            .unwrap()
            .as_deref(),
        Some(second.as_str())
    );

    let (code, _, err) = run(&db, &["routes", "reorder", "claude", &first, &second]);
    assert_eq!(code, 0, "{err}");
    let store = Store::open(&db).unwrap();
    assert_eq!(
        store.primary_provider_id("claude").unwrap().as_deref(),
        Some(first.as_str())
    );
    let by_priority: Vec<String> = store
        .bindings_for_agent("claude")
        .unwrap()
        .into_iter()
        .map(|b| b.provider_id)
        .collect();
    assert_eq!(by_priority, vec![first, second]);
}

#[test]
fn routes_apply_copies_another_agents_route() {
    let (_dir, db) = temp_db();
    let provider = add_provider(&db, "shared", &["claude"]);
    run(&db, &["routes", "strategy", "claude", "failover"]);

    let (code, _, err) = run(
        &db,
        &["routes", "apply", "--from", "claude", "--to", "codex"],
    );
    assert_eq!(code, 0, "{err}");

    let store = Store::open(&db).unwrap();
    assert_eq!(
        store.get_strategy("codex").unwrap().unwrap().kind,
        kiwanod::store::StrategyType::Failover
    );
    assert_eq!(
        store.primary_provider_id("codex").unwrap().as_deref(),
        Some(provider.as_str())
    );
}

// ── agents ──────────────────────────────────────────────────────────────────

fn claude_settings(home: &Path) -> PathBuf {
    home.join(".claude").join("settings.json")
}

fn write_claude_config(home: &Path, body: &str) {
    let dir = home.join(".claude");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("settings.json"), body).unwrap();
}

/// The capability that made a headless server unusable: without a placeholder
/// key the data plane refuses the agent outright, and only the desktop app could
/// mint one.
#[test]
fn takeover_routes_an_agent_and_restore_puts_it_back() {
    let (dir, db) = temp_db();
    let home = dir.path().join("home");
    let home_arg = home.display().to_string();
    let original = r#"{"env":{"ANTHROPIC_BASE_URL":"https://api.anthropic.com","ANTHROPIC_AUTH_TOKEN":"sk-real-abcdef123456"},"other":true}"#;
    write_claude_config(&home, original);

    let (code, _, err) = run(
        &db,
        &[
            "--home",
            &home_arg,
            "providers",
            "add",
            "--name",
            "ds",
            "--endpoint",
            "https://api.deepseek.com/anthropic",
            "--protocol",
            "anthropic",
            "--bind",
            "claude",
        ],
    );
    assert_eq!(code, 0, "{err}");

    let (code, out, err) = run(&db, &["--home", &home_arg, "agents", "takeover", "claude"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("127.0.0.1:8317"), "{out}");

    let rewritten = std::fs::read_to_string(claude_settings(&home)).unwrap();
    assert!(rewritten.contains("http://127.0.0.1:8317"), "{rewritten}");
    assert!(
        !rewritten.contains("sk-real-abcdef123456"),
        "the operator's real key must not survive the takeover: {rewritten}"
    );
    assert!(
        rewritten.contains("other") && rewritten.contains("true"),
        "unrelated settings are preserved: {rewritten}"
    );

    // The key the config carries must be the one the gateway registered —
    // anything else is a 401 waiting to happen.
    let store = Store::open(&db).unwrap();
    let minted = store
        .list_placeholder_keys()
        .unwrap()
        .into_iter()
        .find(|k| k.agent == "claude")
        .expect("a placeholder key was registered");
    assert!(
        rewritten.contains(&minted.key),
        "the config should carry {}: {rewritten}",
        minted.key
    );

    let (code, _, err) = run(&db, &["--home", &home_arg, "agents", "restore", "claude"]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(
        std::fs::read_to_string(claude_settings(&home)).unwrap(),
        original,
        "restore is byte-exact"
    );
}

#[test]
fn takeover_rejects_an_unknown_agent() {
    let (dir, db) = temp_db();
    let home = dir.path().display().to_string();
    let (code, _, err) = run(
        &db,
        &["--home", &home, "agents", "takeover", "not-an-agent"],
    );
    assert_eq!(code, 3);
    assert!(err.contains("unknown agent"), "{err}");
}

#[test]
fn detect_reports_the_known_agents() {
    let (dir, db) = temp_db();
    let home = dir.path().display().to_string();
    let (code, out, err) = run(&db, &["--home", &home, "--json", "agents", "detect"]);
    assert_eq!(code, 0, "{err}");
    let rows: Vec<serde_json::Value> = serde_json::from_str(&out).unwrap();
    let names: Vec<&str> = rows.iter().filter_map(|r| r["agent"].as_str()).collect();
    assert!(names.contains(&"claude"), "{names:?}");
    assert!(names.contains(&"claude-desktop"), "{names:?}");
    // Every row answers the question, whatever the machine has installed.
    assert!(rows.iter().all(|r| r["installed"].is_boolean()), "{rows:?}");
}

// ── logs / dashboard / alerts / gateway ─────────────────────────────────────

fn seed_log(db: &Path, agent: &str, status_code: i64) {
    let store = Store::open(db).unwrap();
    store
        .insert_request_log(&kiwanod::store::RequestLogNew {
            ts: kiwanod::store::now_rfc3339(),
            method: "POST".into(),
            path: "/v1/messages".into(),
            query: None,
            agent: Some(agent.into()),
            attribution: Some("key".into()),
            provider_id: Some("p1".into()),
            model: Some("claude-opus-4-8".into()),
            status_code,
            error_kind: (status_code >= 400).then(|| "upstream_error".to_string()),
            error_message: None,
            session_id: None,
            is_streaming: false,
            input_tokens: 100,
            output_tokens: 20,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            latency_ms: Some(42),
            first_token_ms: None,
            request_headers: None,
            response_headers: None,
            request_body: None,
            response_body: None,
            request_size: 10,
            response_size: 20,
            truncated: false,
            cost: None,
            cost_currency: None,
        })
        .unwrap();
}

#[test]
fn logs_list_filters_by_status_and_reports_the_total() {
    let (_dir, db) = temp_db();
    seed_log(&db, "claude", 200);
    seed_log(&db, "claude", 500);

    let (code, out, err) = run(&db, &["--json", "logs", "list"]);
    assert_eq!(code, 0, "{err}");
    let list: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(list["total"], 2);

    let (_, out, _) = run(&db, &["--json", "logs", "list", "--status", "error"]);
    let list: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(list["total"], 1);
    assert_eq!(list["rows"][0]["status_code"], 500);

    // The column carries a CHECK constraint, so an unknown status should be
    // refused by name rather than by a SQLite constraint message.
    let (code, _, err) = run(&db, &["logs", "list", "--status", "broken"]);
    assert_eq!(code, 2);
    assert!(err.contains("broken"), "{err}");

    let (code, _, err) = run(&db, &["logs", "list", "--page", "0"]);
    assert_eq!(code, 2);
    assert!(err.contains("1-based"), "{err}");
}

#[test]
fn logs_show_reports_a_pruned_row_clearly() {
    let (_dir, db) = temp_db();
    seed_log(&db, "claude", 200);
    let (code, _, err) = run(&db, &["logs", "show", "9999"]);
    assert_eq!(code, 3);
    assert!(err.contains("pruned"), "{err}");
}

#[test]
fn logs_export_writes_the_file_and_requires_no_confirmation() {
    let (dir, db) = temp_db();
    seed_log(&db, "claude", 200);
    let out_path = dir.path().join("logs.csv");
    let (code, _, err) = run(
        &db,
        &["logs", "export", "--out", &out_path.display().to_string()],
    );
    assert_eq!(code, 0, "{err}");
    let csv = std::fs::read_to_string(&out_path).unwrap();
    assert!(csv.contains("claude"), "{csv}");
    assert!(!csv.contains("request_body"), "bodies are opt-in: {csv}");

    let with_bodies = dir.path().join("with-bodies.csv");
    let (code, _, err) = run(
        &db,
        &[
            "logs",
            "export",
            "--out",
            &with_bodies.display().to_string(),
            "--include-bodies",
        ],
    );
    assert_eq!(code, 0, "{err}");
    let csv = std::fs::read_to_string(&with_bodies).unwrap();
    assert!(csv.contains("request_body"), "{csv}");
}

#[test]
fn logs_clear_demands_confirmation() {
    let (_dir, db) = temp_db();
    seed_log(&db, "claude", 200);
    let (code, _, err) = run(&db, &["logs", "clear"]);
    assert_eq!(code, 2, "destructive commands ask first");
    assert!(err.contains("--yes"), "{err}");
    // …and the log is untouched by the refusal.
    let (_, out, _) = run(&db, &["--json", "logs", "list"]);
    let list: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(list["total"], 1);

    let (code, _, err) = run(&db, &["logs", "clear", "--yes"]);
    assert_eq!(code, 0, "{err}");
    let (_, out, _) = run(&db, &["--json", "logs", "list"]);
    let list: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(list["total"], 0);
}

#[test]
fn logs_dir_prints_a_path() {
    let (_dir, db) = temp_db();
    let (code, out, err) = run(&db, &["logs", "dir"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.trim().contains("log"), "{out}");
}

#[test]
fn dashboard_rejects_an_unknown_window() {
    let (_dir, db) = temp_db();
    let (code, _, err) = run(&db, &["dashboard", "--window", "90d"]);
    assert_eq!(code, 2);
    assert!(err.contains("90d"), "{err}");

    let (code, _, err) = run(&db, &["dashboard", "--window", "today"]);
    assert_eq!(code, 0, "{err}");
}

/// Nothing over budget is the answer to the question, not a failure.
#[test]
fn alerts_on_a_quiet_database_succeeds_with_nothing() {
    let (_dir, db) = temp_db();
    let (code, out, err) = run(&db, &["--json", "alerts"]);
    assert_eq!(code, 0, "{err}");
    let alerts: Vec<serde_json::Value> = serde_json::from_str(&out).unwrap();
    assert!(alerts.is_empty());
    // The note is on stderr, so the JSON on stdout stays a bare `[]`.
    assert!(err.contains("no provider is over its allowance"), "{err}");
}

/// `gateway stop` with nothing listening is a failure with a clear message, not
/// a hang or a silent success.
#[test]
fn gateway_stop_reports_when_there_is_nothing_to_stop() {
    let (_dir, db) = temp_db();
    let (code, _, err) = run(&db, &["gateway", "stop"]);
    assert_eq!(code, 3);
    assert!(err.contains("would not stop"), "{err}");
    assert!(
        err.contains("admin"),
        "the message should say where it looked: {err}"
    );
}

// ── settings / config / catalog / import ────────────────────────────────────

#[test]
fn settings_read_and_write_round_trip() {
    let (dir, db) = temp_db();
    // A private home, so the assertion does not depend on what agent configs
    // the machine running the tests happens to have.
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let home_arg = home.display().to_string();

    let (code, out, err) = run(&db, &["--home", &home_arg, "--json", "settings", "get"]);
    assert_eq!(code, 0, "{err}");
    let before: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(before["log_retention_days"], 30, "the default");

    // Values are parsed as JSON, so `false` lands as a boolean and `7` as a
    // number — that is what makes one flag serve every setting.
    let (code, out, err) = run(
        &db,
        &[
            "--home",
            &home_arg,
            "--json",
            "settings",
            "set",
            "--key",
            "cost_alert=false",
            "--key",
            "log_retention_days=7",
        ],
    );
    assert_eq!(code, 0, "{err}");
    let after: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(after["cost_alert"], false);
    assert_eq!(after["log_retention_days"], 7);

    // It persists, rather than only being echoed.
    let (_, out, _) = run(&db, &["--home", &home_arg, "--json", "settings", "get"]);
    let reread: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(reread["cost_alert"], false);
}

#[test]
fn settings_rejects_a_malformed_patch_and_a_bare_key() {
    let (_dir, db) = temp_db();
    let (code, _, err) = run(&db, &["settings", "set", "--patch", "{bad"]);
    assert_eq!(code, 2);
    assert!(err.contains("valid JSON"), "{err}");

    let (code, _, err) = run(&db, &["settings", "set", "--key", "no-equals-sign"]);
    assert_eq!(code, 2);
    assert!(err.contains("KEY=VALUE"), "{err}");

    let (code, _, err) = run(&db, &["settings", "set", "--patch", "[]"]);
    assert_eq!(code, 2);
    assert!(err.contains("object"), "{err}");
}

/// With `--include-keys` the file holds live credentials, so it must not be
/// created world-readable — the database it backs up is 0600 already.
#[test]
fn config_export_is_owner_only_and_round_trips() {
    let (dir, db) = temp_db();
    add_provider(&db, "alpha", &["claude"]);
    let out_path = dir.path().join("config.json");

    let (code, _, err) = run(
        &db,
        &[
            "config",
            "export",
            "--out",
            &out_path.display().to_string(),
            "--include-keys",
        ],
    );
    assert_eq!(code, 0, "{err}");
    assert!(out_path.is_file());

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&out_path).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            mode, 0o600,
            "credentials in a file must not be group-readable"
        );
    }

    // Into a fresh database: the provider and its binding come back.
    let (_dir2, db2) = temp_db();
    let (code, _, err) = run(
        &db2,
        &[
            "config",
            "import",
            "--file",
            &out_path.display().to_string(),
        ],
    );
    assert_eq!(code, 0, "{err}");
    let store = Store::open(&db2).unwrap();
    let providers = store.list_providers().unwrap();
    assert_eq!(providers.len(), 1);
    assert_eq!(providers[0].name, "alpha");
    assert!(
        store.primary_provider_id("claude").unwrap().is_some(),
        "the route came with it"
    );
}

#[test]
fn config_import_reports_a_missing_file() {
    let (_dir, db) = temp_db();
    let (code, _, err) = run(
        &db,
        &["config", "import", "--file", "/nonexistent/config.json"],
    );
    assert_eq!(code, 3);
    assert!(err.contains("cannot read"), "{err}");
}

#[test]
fn catalog_list_filters_by_tag_and_name() {
    let (_dir, db) = temp_db();
    let (code, out, err) = run(&db, &["--json", "catalog", "list"]);
    assert_eq!(code, 0, "{err}");
    let all: serde_json::Value = serde_json::from_str(&out).unwrap();
    let total = all["total"].as_i64().unwrap();
    assert!(total > 0, "the bundled catalog should not be empty");

    let (_, out, _) = run(&db, &["--json", "catalog", "list", "--search", "deepseek"]);
    let filtered: serde_json::Value = serde_json::from_str(&out).unwrap();
    let entries = filtered["entries"].as_array().unwrap();
    assert!(!entries.is_empty());
    assert!(
        entries.iter().all(|e| e["name"]
            .as_str()
            .unwrap()
            .to_lowercase()
            .contains("deepseek")),
        "{entries:?}"
    );

    let (_, out, _) = run(&db, &["--json", "catalog", "list", "--tag", "official"]);
    let tagged: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert!(
        tagged["entries"]
            .as_array()
            .unwrap()
            .iter()
            .all(|e| e["tag"] == "official"),
        "{tagged}"
    );
}

/// Most machines have no cc-switch; that is an answer, not an error.
#[test]
fn cc_switch_import_succeeds_when_there_is_nothing_to_import() {
    let (dir, db) = temp_db();
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let (code, out, err) = run(
        &db,
        &["--home", &home.display().to_string(), "import", "cc-switch"],
    );
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("imported 0"), "{out}");
}

// ── output discipline ───────────────────────────────────────────────────────

/// The defect this rewrite fixes: a mutation under `--json` used to print its
/// reload note to stdout, after the JSON, breaking every `| jq`. The note now
/// goes to stderr and stdout stays parseable.
#[test]
fn json_stdout_stays_parseable_while_notes_go_to_stderr() {
    let (_dir, db) = temp_db();
    let (code, out, err) = run(
        &db,
        &[
            "--json",
            "providers",
            "add",
            "--name",
            "clean",
            "--endpoint",
            "https://clean.example.com",
        ],
    );
    assert_eq!(code, 0, "{err}");
    serde_json::from_str::<serde_json::Value>(&out)
        .unwrap_or_else(|e| panic!("stdout is not a single JSON document ({e}): {out}"));
    assert!(
        err.contains("gateway not reachable"),
        "the reload note belongs on stderr: {err}"
    );
}
