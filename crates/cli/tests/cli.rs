//! End-to-end tests for the command tree.
//!
//! Driven through `run_with` with a temporary database, asserting on the exit
//! code, stdout and stderr separately — the split between the last two is part
//! of the contract, not an implementation detail: stdout is the payload, so a
//! `--json` consumer can pipe it without a diagnostic landing in the middle.
//!
//! Commands that need the daemon are handed one by [`support::serve`]; the rest
//! run with no admin plane beside the temp database, which is a state the CLI
//! has to handle anyway (`migrate.local.md` §14.1, decision D3).

mod support;

use std::path::{Path, PathBuf};

use kiwanod::store::Store;

use support::TestDaemon;

/// Run the CLI against `db` with no daemon, returning (exit code, stdout, stderr).
fn run(db: &Path, args: &[&str]) -> (i32, String, String) {
    run_at(db, None, args)
}

/// Run the CLI against `db` with `daemon` serving its admin plane.
fn run_served(db: &Path, daemon: &TestDaemon, args: &[&str]) -> (i32, String, String) {
    run_at(db, Some(daemon), args)
}

fn run_at(db: &Path, daemon: Option<&TestDaemon>, args: &[&str]) -> (i32, String, String) {
    let mut argv = vec!["--db".to_string(), db.display().to_string()];
    // The flag is how a test names *its* daemon without touching the process
    // environment, which the tests here share and run in parallel.
    if let Some(daemon) = daemon {
        argv.push("--admin-socket".to_string());
        argv.push(daemon.socket());
    }
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
///
/// Seeded through the store, not through the command: `providers add` is served
/// by the daemon now, and a fixture that needed one would drag a daemon into
/// every test that wants a provider to exist. This is the same helper
/// `seed_log` is — the thing under test is what the *command* does with a
/// provider, not how one got there. `providers_add_then_list_roundtrips` is
/// where the command itself is exercised, daemon and all.
fn add_provider(db: &Path, name: &str, bind: &[&str]) -> String {
    let store = Store::open(db).unwrap();
    let input = kiwano_core::vm::NewProviderInput {
        catalog_id: None,
        prices: None,
        name: name.into(),
        api_key: "sk-test".into(),
        endpoint: format!("https://{name}.example.com"),
        protocol: "openai".into(),
        openai_wire: Default::default(),
        model_default: String::new(),
        billing: "payg".into(),
        billing_config: kiwano_core::vm::BillingConfigInput {
            limit_value: None,
            limit_unit: None,
            reset_period: None,
            plan_limits: None,
        },
        agents: Some(bind.iter().map(|a| a.to_string()).collect()),
        endpoints: Vec::new(),
        advanced: None,
        plan_query: None,
    };
    kiwano_core::vm::add_provider(&store, &input)
        .expect("the fixture provider is valid")
        .id
}

// ── globals ─────────────────────────────────────────────────────────────────

/// Globals are position-independent: the hand-rolled parser accepted them
/// anywhere and scripts are written both ways, so clap has to agree.
#[test]
fn globals_are_position_independent() {
    let (_dir, db) = temp_db();
    let daemon = support::serve(&db);
    let (a, out_a, _) = run_served(&db, &daemon, &["providers", "list", "--json"]);
    let (b, out_b, _) = run_served(&db, &daemon, &["--json", "providers", "list"]);
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
    let daemon = support::serve(&db);
    let id = add_provider(&db, "alpha", &[]);

    let (code, out, _) = run_served(&db, &daemon, &["--json", "providers", "list"]);
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
    let (dir, db) = temp_db();
    let daemon = support::serve(&db);
    // A private home: which agents count as bound follows the agent configs, so
    // the assertion must not depend on what the operator has taken over.
    let home = dir.path().join("home");
    let home_arg = home.display().to_string();
    add_provider(&db, "for-claude", &["claude"]);
    add_provider(&db, "unbound", &[]);

    // A binding whose agent never took the gateway over is not a route: the
    // agent's traffic goes wherever its own config points, so the provider is
    // not listed under it.
    let (_, out, _) = run_served(
        &db,
        &daemon,
        &[
            "--home",
            &home_arg,
            "--json",
            "providers",
            "list",
            "--agent",
            "claude",
        ],
    );
    let dormant: Vec<serde_json::Value> = serde_json::from_str(&out).unwrap();
    assert!(dormant.is_empty(), "{dormant:?}");

    // Once the agent is routed through the gateway, its provider is listed.
    write_claude_config(
        &home,
        r#"{"env":{"ANTHROPIC_AUTH_TOKEN":"kw-ag-claude-test"}}"#,
    );
    let (_, out, _) = run_served(
        &db,
        &daemon,
        &[
            "--home",
            &home_arg,
            "--json",
            "providers",
            "list",
            "--agent",
            "claude",
        ],
    );
    let list: Vec<serde_json::Value> = serde_json::from_str(&out).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["name"], "for-claude");
    assert_eq!(list[0]["agents"], serde_json::json!(["claude"]));
    assert_eq!(list[0]["serving_agents"], serde_json::json!(["claude"]));
}

/// Billing words from the old CLI keep working, and both spellings land in the
/// store as the same row.
#[test]
fn billing_accepts_both_vocabularies() {
    let (_dir, db) = temp_db();
    let daemon = support::serve(&db);
    let (code, out, err) = run_served(
        &db,
        &daemon,
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

    let (code, _, err) = run_served(
        &db,
        &daemon,
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
    let daemon = support::serve(&db);
    let first = add_provider(&db, "first", &["claude"]);
    let second = add_provider(&db, "second", &[]);

    let (code, _, err) = run_served(
        &db,
        &daemon,
        &["providers", "use", &second, "--agent", "claude"],
    );
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
    let daemon = support::serve(&db);
    let (code, _, err) = run_served(
        &db,
        &daemon,
        &["providers", "use", "nope", "--agent", "claude"],
    );
    assert_eq!(code, 3, "a runtime error, not a usage error");
    // The daemon's own wording, which names the id — the row is on its side, so
    // so is the sentence about not finding it.
    assert!(err.contains("not found"), "{err}");
    assert!(err.contains("nope"), "{err}");
}

#[test]
fn providers_remove_promotes_the_next_candidate() {
    let (_dir, db) = temp_db();
    let daemon = support::serve(&db);
    let first = add_provider(&db, "first", &["claude"]);
    let second = add_provider(&db, "second", &[]);
    run_served(
        &db,
        &daemon,
        &["providers", "use", &second, "--agent", "claude"],
    );

    let (code, _, err) = run_served(&db, &daemon, &["providers", "remove", &second]);
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
    let daemon = support::serve(&db);
    let (code, out, err) = run_served(
        &db,
        &daemon,
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
    let daemon = support::serve(&db);
    let id = add_provider(&db, "azure", &[]);
    run_served(
        &db,
        &daemon,
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
    let (code, _, err) = run_served(
        &db,
        &daemon,
        &["providers", "edit", &id, "--name", "renamed"],
    );
    assert_eq!(code, 0, "{err}");

    let store = Store::open(&db).unwrap();
    let row = store.get_provider(&id).unwrap().unwrap();
    assert_eq!(row.name, "renamed");
    assert_eq!(row.timeout_secs, Some(120), "timeout survived a rename");
    assert!(row.headers.is_some(), "headers survived a rename");
    assert_eq!(row.endpoints.len(), 1, "endpoints survived a rename");

    // Changing one field still leaves the other two alone.
    let (code, _, err) = run_served(&db, &daemon, &["providers", "edit", &id, "--retries", "3"]);
    assert_eq!(code, 0, "{err}");
    let store = Store::open(&db).unwrap();
    let row = store.get_provider(&id).unwrap().unwrap();
    assert_eq!(row.retries, Some(3));
    assert_eq!(row.timeout_secs, Some(120));

    // …and clearing them is explicit, not a side effect of an unrelated edit.
    let (code, _, err) = run_served(&db, &daemon, &["providers", "edit", &id, "--no-headers"]);
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
    let daemon = support::serve(&db);
    let (code, _, err) = run_served(
        &db,
        &daemon,
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

    let (code, _, err) = run_served(
        &db,
        &daemon,
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
    let daemon = support::serve(&db);
    let (code, out, err) = run_served(
        &db,
        &daemon,
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
    let (code, _, err) = run_served(
        &db,
        &daemon,
        &["providers", "edit", &id, "--plan-query", "not json"],
    );
    assert_eq!(code, 2);
    assert!(err.contains("valid JSON"), "{err}");
    let (code, _, err) = run_served(
        &db,
        &daemon,
        &["providers", "edit", &id, "--plan-query", "[]"],
    );
    assert_eq!(code, 2);
    assert!(err.contains("JSON object"), "{err}");

    // Clearing is explicit — an absent --plan-query means "keep".
    let (code, _, err) = run_served(&db, &daemon, &["providers", "edit", &id, "--name", "kimi2"]);
    assert_eq!(code, 0, "{err}");
    let store = Store::open(&db).unwrap();
    assert!(store
        .get_provider(&id)
        .unwrap()
        .unwrap()
        .plan_query
        .is_some());

    let (code, _, err) = run_served(
        &db,
        &daemon,
        &["providers", "edit", &id, "--clear-plan-query"],
    );
    assert_eq!(code, 0, "{err}");
    let store = Store::open(&db).unwrap();
    assert_eq!(store.get_provider(&id).unwrap().unwrap().plan_query, None);

    // And with none configured, quota says so rather than inventing an answer.
    let (code, _, err) = run_served(&db, &daemon, &["providers", "quota", &id]);
    assert_eq!(code, 3);
    assert!(err.to_lowercase().contains("plan query"), "{err}");
}

// ── keys ────────────────────────────────────────────────────────────────────

#[test]
fn keys_add_list_remove_roundtrips() {
    let (_dir, db) = temp_db();
    let daemon = support::serve(&db);
    let provider = add_provider(&db, "rotating", &[]);

    let (code, out, err) = run_served(
        &db,
        &daemon,
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

    let (_, out, _) = run_served(&db, &daemon, &["--json", "keys", "list", &provider]);
    let keys: Vec<serde_json::Value> = serde_json::from_str(&out).unwrap();
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0]["label"], "spare");

    let (code, _, err) = run_served(&db, &daemon, &["keys", "remove", &key_id.to_string()]);
    assert_eq!(code, 0, "{err}");
    let (_, out, _) = run_served(&db, &daemon, &["--json", "keys", "list", &provider]);
    let keys: Vec<serde_json::Value> = serde_json::from_str(&out).unwrap();
    assert!(keys.is_empty());
}

/// A command that has moved answers **only** through the daemon, and says so
/// when there is none.
///
/// This is the evidence that it moved at all. While the command read the store
/// itself it answered with no daemon running — and a CLI that still did would
/// pass every test above, because the temp database is right there. Now the
/// answer comes over the admin plane, so a missing daemon is an error rather
/// than a silent second source of the same data (`migrate.local.md` §14.1,
/// decision D1). When the next batch moves, add its command here.
#[test]
fn a_moved_command_needs_the_daemon() {
    let (_dir, db) = temp_db();
    let provider = add_provider(&db, "rotating", &[]);

    for args in [
        vec!["keys", "list", provider.as_str()],
        vec!["keys", "add", provider.as_str(), "--key", "sk-another"],
        vec!["routes", "list"],
        vec!["routes", "binding", "add", "claude", provider.as_str()],
        vec!["routes", "apply", "--from", "claude", "--to", "codex"],
        vec!["logs", "list"],
        vec!["logs", "show", "1"],
        vec!["logs", "clear", "--yes"],
        vec!["sessions"],
        vec!["dashboard", "--window", "today"],
        vec!["alerts"],
        vec!["agents", "add", "--name", "night batch"],
        vec!["agents", "remove", "no-such-agent"],
        vec!["providers", "remove", provider.as_str()],
        vec!["providers", "use", provider.as_str(), "--agent", "claude"],
        vec!["providers", "disable", provider.as_str()],
        vec!["providers", "edit", provider.as_str(), "--name", "renamed"],
        vec!["catalog", "list"],
        vec!["catalog", "currency"],
        vec!["usage", "--days", "7"],
        vec!["insights", "--days", "7"],
        vec!["cache-experiment", "--days", "7"],
        vec!["rules", "apply", "claude"],
    ] {
        let (code, out, err) = run(&db, &args);
        assert_eq!(code, 3, "{args:?}\nstdout: {out}\nstderr: {err}");
        assert!(err.contains("gateway is not answering"), "{args:?}: {err}");
    }
}

/// The takeover's store half is the daemon's now — it was the last command that
/// wrote the shared database directly (`migrate.local.md` §10.43).
///
/// Its own test rather than a row in the table above, because it can assert one
/// thing more: **the file half does not run when the store half never landed.**
/// That ordering is §8's, and it is what keeps a half-done takeover pointing at
/// a gateway that knows its key rather than at one that would refuse it.
#[test]
fn a_takeover_needs_the_daemon_and_writes_no_config_without_one() {
    let (dir, db) = temp_db();
    let home = dir.path().join("home");
    write_claude_config(
        &home,
        r#"{"env":{"ANTHROPIC_BASE_URL":"https://api.anthropic.com","ANTHROPIC_AUTH_TOKEN":"sk-real-1"}}"#,
    );
    let before = std::fs::read_to_string(claude_settings(&home)).unwrap();

    let (code, _, err) = run(
        &db,
        &[
            "--home",
            &home.display().to_string(),
            "agents",
            "takeover",
            "claude",
        ],
    );
    assert_eq!(code, 3, "{err}");
    assert!(err.contains("gateway is not answering"), "{err}");
    assert_eq!(
        std::fs::read_to_string(claude_settings(&home)).unwrap(),
        before,
        "the agent's config must be untouched when the store half did not land"
    );
}

// ── usage / status ──────────────────────────────────────────────────────────

#[test]
fn usage_reports_an_empty_window_without_failing() {
    let (_dir, db) = temp_db();
    let daemon = support::serve(&db);
    let (code, out, err) = run_served(&db, &daemon, &["usage"]);
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

/// The daemon harness, proven end to end.
///
/// `status` is the one command that already spoke to the admin plane before this
/// migration, which makes it the honest proof that the harness serves the
/// *real* router: if `support::serve` bound nothing, or bound something the CLI
/// does not resolve, this test would land in the "not running" branch below
/// instead of passing. Every command moved onto the API in the next step leans
/// on exactly this.
#[test]
fn the_harness_serves_a_daemon_the_cli_actually_reaches() {
    let (_dir, db) = temp_db();
    let daemon = support::serve(&db);
    let (code, out, err) = run_served(&db, &daemon, &["status"]);
    assert_eq!(code, 0, "stdout: {out}\nstderr: {err}");
    assert!(
        out.contains("gateway: running"),
        "the CLI did not reach the harness's daemon: {out}"
    );
}

/// Gateway down is an answer, not a failure — exit 1, and that is *all* it says.
///
/// It used to add a report read from the shared database. That is gone on
/// purpose (`migrate.local.md` §14.1, decision D3): a client answering from a
/// database of its own is answering about a different daemon the moment it is
/// pointed at another machine, and a remote one has no such file to read.
#[test]
fn status_without_a_gateway_exits_1_and_says_only_that() {
    let (_dir, db) = temp_db();
    add_provider(&db, "alpha", &[]);
    let (code, out, err) = run(&db, &["status"]);
    assert_eq!(code, 1, "{err}");
    assert!(out.contains("gateway: not running"), "{out}");
    assert!(
        !out.contains("store:"),
        "the client must not answer from a database of its own: {out}"
    );
}

/// The other half of it: with a daemon, the footer's totals come from *it*.
///
/// The rows are seeded directly, which is what the daemon reads; the numbers
/// arriving in the report is the evidence they travelled over the admin plane.
#[test]
fn status_reports_the_daemons_totals_when_it_answers() {
    let (_dir, db) = temp_db();
    let daemon = support::serve(&db);
    add_provider(&db, "alpha", &[]);
    // Today's totals come from the *usage* table — what was billable — not from
    // the request log, so seeding a log row would not move them.
    {
        let store = Store::open(&db).unwrap();
        store
            .record_usage(&kiwanod::store::UsageRecord {
                ts: kiwanod::store::now_rfc3339(),
                agent: "claude".into(),
                provider_id: Some("alpha".into()),
                client_key_id: None,
                model: None,
                input_tokens: 100,
                output_tokens: 20,
                cache_read_tokens: 0,
                cache_creation_tokens: 0,
                latency_ms: None,
                status: "ok".into(),
                cost: None,
                cost_currency: None,
                cost_off_peak: None,
                project: None,
                session_id: None,
                import_key: None,
            })
            .unwrap();
    }
    let (code, out, err) = run_served(&db, &daemon, &["status"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("gateway: running"), "{out}");
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
    let daemon = support::serve(&db);
    let id = add_provider(&db, "original", &["claude"]);

    let (code, out, err) = run_served(
        &db,
        &daemon,
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
    let daemon = support::serve(&db);
    let id = add_provider(&db, "bound", &["claude"]);
    let (code, _, err) = run_served(&db, &daemon, &["providers", "edit", &id, "--no-bind"]);
    assert_eq!(code, 0, "{err}");
    let store = Store::open(&db).unwrap();
    assert!(store.bindings_for_agent("claude").unwrap().is_empty());
}

/// A provider with no plan query has no quota to report, and saying so beats a
/// fabricated 0%.
#[test]
fn providers_quota_reports_a_missing_plan_query() {
    let (_dir, db) = temp_db();
    let daemon = support::serve(&db);
    let id = add_provider(&db, "plain", &[]);
    let (code, out, err) = run_served(&db, &daemon, &["providers", "quota", &id]);
    assert_eq!(code, 3);
    assert!(err.to_lowercase().contains("plan query"), "{err}");
    assert!(out.is_empty(), "no payload on stdout for a failure: {out}");
}

/// Parking a provider, and putting it back: the row, its key and its route all
/// survive, and only whether it may serve changes.
#[test]
fn providers_disable_and_enable_round_trip() {
    let (dir, db) = temp_db();
    let daemon = support::serve(&db);
    let home = dir.path().join("home");
    let home_arg = home.display().to_string();
    let id = add_provider(&db, "alpha", &["claude"]);

    let (code, out, err) = run_served(&db, &daemon, &["providers", "disable", &id]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("fall through"), "{out}");
    assert!(out.contains("claude"), "{out}");

    // Parked, not gone: the row is still there, with its key, and says which
    // state it is in.
    let (_, out, _) = run_served(
        &db,
        &daemon,
        &["--home", &home_arg, "--json", "providers", "list"],
    );
    let list: Vec<serde_json::Value> = serde_json::from_str(&out).unwrap();
    assert_eq!(list.len(), 1, "{list:?}");
    assert_eq!(list[0]["enabled"], false);
    let store = Store::open(&db).unwrap();
    assert_eq!(
        store.bindings_for_agent("claude").unwrap().len(),
        1,
        "the route is untouched"
    );

    let (code, out, err) = run_served(&db, &daemon, &["providers", "enable", &id]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("may serve again"), "{out}");
    let (_, out, _) = run_served(
        &db,
        &daemon,
        &["--home", &home_arg, "--json", "providers", "list"],
    );
    let list: Vec<serde_json::Value> = serde_json::from_str(&out).unwrap();
    assert_eq!(list[0]["enabled"], true);

    // An unknown id is an error rather than a quiet no-op.
    let (code, _, err) = run_served(&db, &daemon, &["providers", "disable", "ghost"]);
    assert_eq!(code, 3, "{err}");
    assert!(err.contains("not found"), "{err}");
}

// ── routes ──────────────────────────────────────────────────────────────────

#[test]
fn strategy_roundrobin_seeds_balanced_weights() {
    let (_dir, db) = temp_db();
    let daemon = support::serve(&db);
    add_provider(&db, "one", &["claude"]);
    add_provider(&db, "two", &["claude"]);

    let (code, _, err) = run_served(
        &db,
        &daemon,
        &["routes", "strategy", "claude", "roundrobin"],
    );
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
    let daemon = support::serve(&db);
    add_provider(&db, "one", &["claude"]);

    let (code, _, err) = run_served(&db, &daemon, &["routes", "strategy", "claude", "quota"]);
    assert_eq!(code, 2);
    assert!(err.contains("--limit"), "{err}");

    let (code, _, err) = run_served(
        &db,
        &daemon,
        &[
            "routes", "strategy", "claude", "quota", "--limit", "10", "--unit", "cost",
        ],
    );
    assert_eq!(code, 2);
    assert!(err.contains("cost"), "{err}");

    let (code, _, err) = run_served(
        &db,
        &daemon,
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
    let (code, _, err) = run_served(
        &db,
        &daemon,
        &["routes", "strategy", "claude", "failover", "--limit", "10"],
    );
    assert_eq!(code, 2);
    assert!(err.contains("quota"), "{err}");
}

#[test]
fn routes_binding_add_set_and_remove() {
    let (_dir, db) = temp_db();
    let daemon = support::serve(&db);
    let first = add_provider(&db, "first", &["claude"]);
    let second = add_provider(&db, "second", &[]);

    let (code, _, err) = run_served(
        &db,
        &daemon,
        &["routes", "binding", "add", "claude", &second],
    );
    assert_eq!(code, 0, "{err}");
    let store = Store::open(&db).unwrap();
    assert_eq!(store.bindings_for_agent("claude").unwrap().len(), 2);

    let (code, _, err) = run_served(
        &db,
        &daemon,
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

    let (code, _, err) = run_served(
        &db,
        &daemon,
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
    let (code, _, err) = run_served(
        &db,
        &daemon,
        &[
            "routes", "binding", "set", "claude", &second, "--window", "9am-5pm",
        ],
    );
    assert_eq!(code, 2);
    assert!(err.contains("HH:MM"), "{err}");

    let (code, _, err) = run_served(
        &db,
        &daemon,
        &["routes", "binding", "remove", "claude", &second],
    );
    assert_eq!(code, 0, "{err}");
    let store = Store::open(&db).unwrap();
    let remaining = store.bindings_for_agent("claude").unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].provider_id, first);
}

#[test]
fn routes_reorder_rewrites_priorities() {
    let (_dir, db) = temp_db();
    let daemon = support::serve(&db);
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

    let (code, _, err) = run_served(
        &db,
        &daemon,
        &["routes", "reorder", "claude", &first, &second],
    );
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
    let daemon = support::serve(&db);
    let provider = add_provider(&db, "shared", &["claude"]);
    run_served(&db, &daemon, &["routes", "strategy", "claude", "failover"]);

    let (code, _, err) = run_served(
        &db,
        &daemon,
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
    let daemon = support::serve(&db);
    let home = dir.path().join("home");
    let home_arg = home.display().to_string();
    let original = r#"{"env":{"ANTHROPIC_BASE_URL":"https://api.anthropic.com","ANTHROPIC_AUTH_TOKEN":"sk-real-abcdef123456"},"other":true}"#;
    write_claude_config(&home, original);

    let (code, _, err) = run_served(
        &db,
        &daemon,
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

    let (code, out, err) = run_served(
        &db,
        &daemon,
        &["--home", &home_arg, "agents", "takeover", "claude"],
    );
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
        .list_client_keys()
        .unwrap()
        .into_iter()
        .find(|k| k.agent == "claude")
        .expect("a placeholder key was registered");
    assert!(
        rewritten.contains(&minted.key),
        "the config should carry {}: {rewritten}",
        minted.key
    );

    let (code, _, err) = run_served(
        &db,
        &daemon,
        &["--home", &home_arg, "agents", "restore", "claude"],
    );
    assert_eq!(code, 0, "{err}");
    assert_eq!(
        std::fs::read_to_string(claude_settings(&home)).unwrap(),
        original,
        "restore is byte-exact"
    );

    // The route went with the takeover — the agent is nobody's candidate — while
    // the provider itself stays in the list, unbound: it is the user's row, and
    // the next takeover imports and binds it again.
    assert!(store.bindings_for_agent("claude").unwrap().is_empty());
    assert!(store.get_strategy("claude").unwrap().is_none());
    let (code, out, err) = run_served(
        &db,
        &daemon,
        &["--home", &home_arg, "--json", "providers", "list"],
    );
    assert_eq!(code, 0, "{err}");
    let list: Vec<serde_json::Value> = serde_json::from_str(&out).unwrap();
    // Two rows: `ds`, plus the provider the first takeover imported out of the
    // agent's own config. Both are still there, both unbound.
    assert_eq!(list.len(), 2, "{list:?}");
    let ds = list.iter().find(|p| p["name"] == "ds").expect("ds");
    assert_eq!(ds["agents"], serde_json::json!([]));
    assert!(list
        .iter()
        .all(|p| p["serving_agents"] == serde_json::json!([])));
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

/// The whole life of a user-defined agent from the command line: it is made
/// with a name, it lists with the built-ins, a provider is bound to it with the
/// existing route commands, and deleting it takes the route and the key with it
/// while the history stays.
#[test]
fn agents_add_list_bind_and_remove_a_custom_agent() {
    let (dir, db) = temp_db();
    let daemon = support::serve(&db);
    let home = dir.path().join("home");
    let home_arg = home.display().to_string();

    let (code, out, err) = run_served(
        &db,
        &daemon,
        &[
            "--json",
            "agents",
            "add",
            "--name",
            "Long Tasks",
            "--note",
            "night batch",
            "--protocol",
            "gemini",
        ],
    );
    assert_eq!(code, 0, "{err}");
    let created: serde_json::Value = serde_json::from_str(&out).unwrap();
    let id = created["id"].as_str().unwrap().to_string();
    let key = created["placeholder_key"].as_str().unwrap().to_string();
    assert!(id.starts_with("long-tasks-"), "{id}");
    assert!(key.starts_with(&format!("kw-ag-{id}")), "{key}");
    assert_eq!(created["protocol"], "gemini");

    // It lists as an agent, marked custom and routed (there is no config for it
    // to fail to point at the gateway).
    let (_, out, _) = run_served(
        &db,
        &daemon,
        &["--home", &home_arg, "--json", "agents", "list"],
    );
    let rows: Vec<serde_json::Value> = serde_json::from_str(&out).unwrap();
    let mine = rows
        .iter()
        .find(|r| r["id"] == id.as_str())
        .unwrap_or_else(|| panic!("{id} is not in {rows:?}"));
    assert_eq!(mine["kind"], "custom");
    assert_eq!(mine["routed"], true);
    assert_eq!(mine["label"], "Long Tasks");
    assert_eq!(mine["note"], "night batch");
    assert_eq!(mine["protocols"], "gemini", "as it was defined");

    // A built-in reports the protocols its clients speak — a list, since some
    // carry more than one. The gateway routes by key and never reads this.
    let claude = rows.iter().find(|r| r["id"] == "claude").unwrap();
    assert_eq!(claude["protocols"], "anthropic");

    // Binding is the existing command: a custom agent is an agent id like any
    // other to everything that reads routes.
    let provider_id = add_provider(&db, "alpha", &[]);
    let (code, _, err) = run_served(
        &db,
        &daemon,
        &["routes", "binding", "add", &id, &provider_id],
    );
    assert_eq!(code, 0, "{err}");
    let (_, out, _) = run_served(
        &db,
        &daemon,
        &["--home", &home_arg, "--json", "providers", "list"],
    );
    let providers: Vec<serde_json::Value> = serde_json::from_str(&out).unwrap();
    assert_eq!(
        providers[0]["agents"],
        serde_json::json!([id.clone()]),
        "the provider is listed under the custom agent"
    );

    // Deleting it clears the route and the key; the provider row stays.
    let (code, out, err) = run_served(&db, &daemon, &["agents", "remove", &id]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("usage history stays"), "{out}");
    let store = Store::open(&db).unwrap();
    assert!(store.bindings_for_agent(&id).unwrap().is_empty());
    assert!(store.get_strategy(&id).unwrap().is_none());
    assert!(store
        .list_client_keys()
        .unwrap()
        .iter()
        .all(|k| k.agent != id));
    assert!(store.get_provider(&provider_id).unwrap().is_some());

    // …and a second removal has nothing to remove.
    let (code, _, err) = run_served(&db, &daemon, &["agents", "remove", &id]);
    assert_eq!(code, 3, "{err}");
    assert!(err.contains("no such custom agent"), "{err}");
}

#[test]
fn agents_add_requires_a_name() {
    let (_dir, db) = temp_db();
    let daemon = support::serve(&db);
    let (code, _, err) = run_served(&db, &daemon, &["agents", "add", "--name", "   "]);
    assert_eq!(code, 3, "{err}");
    assert!(err.contains("needs a name"), "{err}");
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
            reasoning_tokens: 0,
            usage_missing: false,
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
            cost_off_peak: None,
            request_notes: None,
        })
        .unwrap();
}

/// A traffic row under a session id — what the sessions merge keys on, and what
/// `logs list --session` filters by.
fn seed_session_log(
    db: &Path,
    agent: &str,
    session: &str,
    cost: Option<f64>,
    currency: Option<&str>,
) {
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
            status_code: 200,
            error_kind: None,
            error_message: None,
            session_id: Some(session.into()),
            is_streaming: false,
            input_tokens: 100,
            output_tokens: 20,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            reasoning_tokens: 0,
            usage_missing: false,
            latency_ms: Some(42),
            first_token_ms: None,
            request_headers: None,
            response_headers: None,
            request_body: None,
            response_body: None,
            request_size: 10,
            response_size: 20,
            truncated: false,
            cost,
            cost_currency: currency.map(str::to_string),
            cost_off_peak: None,
            request_notes: None,
        })
        .unwrap();
}

/// An imported session as `kiwano history import` would have written it: the
/// project, the turns and the tools, which only the agent's own file knows.
fn seed_imported_session(db: &Path, agent: &str, session: &str, project: &str) {
    let store = Store::open(db).unwrap();
    store
        .upsert_imported_sessions(&[kiwanod::store::ImportedSession {
            import_key: format!("{agent}:{session}"),
            agent: agent.into(),
            project: Some(project.into()),
            session_id: session.into(),
            started_at: "2026-01-01T00:00:00+00:00".into(),
            ended_at: "2026-01-01T02:00:00+00:00".into(),
            turns: 5,
            input_tokens: 900,
            output_tokens: 90,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            tool_calls: Some(r#"[{"name":"Bash","count":2}]"#.into()),
            skills: None,
        }])
        .unwrap();
}

#[test]
fn logs_list_filters_by_status_and_reports_the_total() {
    let (_dir, db) = temp_db();
    let daemon = support::serve(&db);
    seed_log(&db, "claude", 200);
    seed_log(&db, "claude", 500);

    let (code, out, err) = run_served(&db, &daemon, &["--json", "logs", "list"]);
    assert_eq!(code, 0, "{err}");
    let list: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(list["total"], 2);

    let (_, out, _) = run_served(
        &db,
        &daemon,
        &["--json", "logs", "list", "--status", "error"],
    );
    let list: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(list["total"], 1);
    assert_eq!(list["rows"][0]["status_code"], 500);

    // The column carries a CHECK constraint, so an unknown status should be
    // refused by name rather than by a SQLite constraint message.
    let (code, _, err) = run_served(&db, &daemon, &["logs", "list", "--status", "broken"]);
    assert_eq!(code, 2);
    assert!(err.contains("broken"), "{err}");

    let (code, _, err) = run_served(&db, &daemon, &["logs", "list", "--page", "0"]);
    assert_eq!(code, 2);
    assert!(err.contains("1-based"), "{err}");
}

/// `--session` narrows the trail to the requests one session named — the same
/// id `kiwano sessions` lists, so a row there can be opened here.
#[test]
fn logs_list_filters_by_the_session_an_agent_named() {
    let (_dir, db) = temp_db();
    let daemon = support::serve(&db);
    seed_session_log(&db, "claude", "s-1", None, None);
    seed_session_log(&db, "claude", "s-1", None, None);
    seed_session_log(&db, "claude", "s-2", None, None);
    // A request nobody attributed to a session: it is in no session's list.
    seed_log(&db, "claude", 200);

    let (code, out, err) = run_served(
        &db,
        &daemon,
        &["--json", "logs", "list", "--session", "s-1"],
    );
    assert_eq!(code, 0, "{err}");
    let list: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(list["total"], 2, "{list}");
    assert!(
        list["rows"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["session_id"] == "s-1"),
        "{list}"
    );
}

/// One session id in both ledgers is one row that says `both`, and each field
/// comes from the side that can know it. The two are **not** added: the traffic
/// numbers win where both have one.
#[test]
fn sessions_merge_the_gateways_traffic_with_the_agents_own_files() {
    let (_dir, db) = temp_db();
    let daemon = support::serve(&db);
    // `s-1` is in both: the gateway routed two of its requests, one of them
    // priced, and the agent's file has the project and the turns.
    seed_session_log(&db, "claude", "s-1", Some(2.5), Some("USD"));
    seed_session_log(&db, "claude", "s-1", None, None);
    // `s-2` only the gateway saw; `s-3` only the file has.
    seed_session_log(&db, "codex", "s-2", Some(1.0), Some("USD"));
    seed_imported_session(&db, "claude", "s-1", "acme-api");
    seed_imported_session(&db, "claude", "s-3", "other-repo");

    let (code, out, err) = run_served(&db, &daemon, &["--json", "sessions"]);
    assert_eq!(code, 0, "{err}");
    let rows: Vec<serde_json::Value> = serde_json::from_str(&out).unwrap();
    let by_id = |id: &str| {
        rows.iter()
            .find(|r| r["session_id"] == id)
            .unwrap_or_else(|| panic!("no {id} in {rows:?}"))
            .clone()
    };

    let both = by_id("s-1");
    assert_eq!(both["source"], "both");
    assert_eq!(both["project"], "acme-api", "the files' project wins");
    assert_eq!(both["turns"], 5, "only the files count turns");
    assert_eq!(both["requests"], 2, "only the traffic counts requests");
    assert_eq!(
        both["input_tokens"], 200,
        "the traffic's metered tokens, not the file's 900 added to them: {both}"
    );
    assert_eq!(both["unpriced_rows"], 1, "the costless row is counted");
    assert_eq!(both["cost"][0]["currency"], "USD");
    assert_eq!(both["cost"][0]["amount"], 2.5);
    assert_eq!(both["tool_calls"][0]["name"], "Bash");

    let gateway = by_id("s-2");
    assert_eq!(gateway["source"], "gateway");
    assert!(gateway["project"].is_null(), "no file placed it: {gateway}");
    assert_eq!(gateway["requests"], 1);
    assert_eq!(gateway["turns"], 0);

    let imported = by_id("s-3");
    assert_eq!(imported["source"], "imported");
    assert_eq!(imported["requests"], 0);
    assert_eq!(imported["turns"], 5);
    assert!(
        imported["cost"].as_array().unwrap().is_empty(),
        "the files carry tokens, not money: {imported}"
    );

    // The filters are exact, and `--project` is the files' dimension: a session
    // with no project cannot be said to match one.
    let (_code, out, _) = run_served(
        &db,
        &daemon,
        &["--json", "sessions", "--project", "acme-api"],
    );
    let placed: Vec<serde_json::Value> = serde_json::from_str(&out).unwrap();
    assert_eq!(placed.len(), 1);
    assert_eq!(placed[0]["session_id"], "s-1");

    // And the table renders without the JSON.
    let (code, out, err) = run_served(&db, &daemon, &["sessions"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("SOURCE"), "{out}");
    assert!(out.contains("both"), "{out}");
    assert!(out.contains("2.50 USD"), "{out}");
}

#[test]
fn logs_show_reports_a_pruned_row_clearly() {
    let (_dir, db) = temp_db();
    let daemon = support::serve(&db);
    seed_log(&db, "claude", 200);
    let (code, _, err) = run_served(&db, &daemon, &["logs", "show", "9999"]);
    assert_eq!(code, 3);
    assert!(err.contains("pruned"), "{err}");
}

#[test]
fn logs_export_writes_the_file_and_requires_no_confirmation() {
    let (dir, db) = temp_db();
    let daemon = support::serve(&db);
    seed_log(&db, "claude", 200);
    let out_path = dir.path().join("logs.csv");
    let (code, _, err) = run_served(
        &db,
        &daemon,
        &["logs", "export", "--out", &out_path.display().to_string()],
    );
    assert_eq!(code, 0, "{err}");
    let csv = std::fs::read_to_string(&out_path).unwrap();
    assert!(csv.contains("claude"), "{csv}");
    // The bodies ride along — there is no flag that withholds them — and the
    // header says so.
    assert!(csv.contains("request_body,response_body"), "{csv}");
}

#[test]
fn logs_clear_demands_confirmation() {
    let (_dir, db) = temp_db();
    let daemon = support::serve(&db);
    seed_log(&db, "claude", 200);
    let (code, _, err) = run_served(&db, &daemon, &["logs", "clear"]);
    assert_eq!(code, 2, "destructive commands ask first");
    assert!(err.contains("--yes"), "{err}");
    // …and the log is untouched by the refusal.
    let (_, out, _) = run_served(&db, &daemon, &["--json", "logs", "list"]);
    let list: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(list["total"], 1);

    let (code, _, err) = run_served(&db, &daemon, &["logs", "clear", "--yes"]);
    assert_eq!(code, 0, "{err}");
    let (_, out, _) = run_served(&db, &daemon, &["--json", "logs", "list"]);
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
    let daemon = support::serve(&db);
    let (code, _, err) = run_served(&db, &daemon, &["dashboard", "--window", "90d"]);
    assert_eq!(code, 2);
    assert!(err.contains("90d"), "{err}");

    let (code, _, err) = run_served(&db, &daemon, &["dashboard", "--window", "today"]);
    assert_eq!(code, 0, "{err}");

    // "all" is a window like the others, not an omitted `--window`.
    let (code, _, err) = run_served(&db, &daemon, &["dashboard", "--window", "all"]);
    assert_eq!(code, 0, "{err}");
}

/// Nothing over budget is the answer to the question, not a failure.
#[test]
fn alerts_on_a_quiet_database_succeeds_with_nothing() {
    let (_dir, db) = temp_db();
    let daemon = support::serve(&db);
    let (code, out, err) = run_served(&db, &daemon, &["--json", "alerts"]);
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
    let daemon = support::serve(&db);
    // A private home, so the assertion does not depend on what agent configs
    // the machine running the tests happens to have.
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let home_arg = home.display().to_string();

    let (code, out, err) = run_served(
        &db,
        &daemon,
        &["--home", &home_arg, "--json", "settings", "get"],
    );
    assert_eq!(code, 0, "{err}");
    let before: serde_json::Value = serde_json::from_str(&out).unwrap();
    // 0 is "keep every row", and it is the default: capture is the point of
    // the feature, so nothing prunes the log until someone asks it to.
    assert_eq!(before["log_retention_days"], 0, "the default");

    // Values are parsed as JSON, so `false` lands as a boolean and `7` as a
    // number — that is what makes one flag serve every setting.
    let (code, out, err) = run_served(
        &db,
        &daemon,
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
            "--key",
            "log_max_body_bytes=1048576",
        ],
    );
    assert_eq!(code, 0, "{err}");
    let after: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(after["cost_alert"], false);
    assert_eq!(after["log_retention_days"], 7);
    assert_eq!(after["log_max_body_bytes"], 1048576);

    // It persists, rather than only being echoed.
    let (_, out, _) = run_served(
        &db,
        &daemon,
        &["--home", &home_arg, "--json", "settings", "get"],
    );
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
    let daemon = support::serve(&db);
    add_provider(&db, "alpha", &["claude"]);
    let out_path = dir.path().join("config.json");

    let (code, _, err) = run_served(
        &db,
        &daemon,
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
    let daemon2 = support::serve(&db2);
    let (code, _, err) = run_served(
        &db2,
        &daemon2,
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
    let daemon = support::serve(&db);
    // The shelf reads the Hub cache and nothing else — there is no bundled
    // catalog — so the fixture is the cache. That makes this deterministic
    // instead of depending on whatever blob a release happened to compile in.
    {
        // Seeded through the store: the cache is the daemon's table, so its
        // accessor there is the only one — the `Aux` copies are gone
        // (`migrate.local.md` §10.14).
        let store = Store::open(&db).unwrap();
        store
            .save_hub_cache(SEED_CATALOG, "2026-09-07T00:00:00Z")
            .unwrap();
    }

    let (code, out, err) = run_served(&db, &daemon, &["--json", "catalog", "list"]);
    assert_eq!(code, 0, "{err}");
    let all: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(all["total"], 2);

    let (_, out, _) = run_served(
        &db,
        &daemon,
        &["--json", "catalog", "list", "--search", "deepseek"],
    );
    let filtered: serde_json::Value = serde_json::from_str(&out).unwrap();
    let entries = filtered["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert!(
        entries.iter().all(|e| e["name"]
            .as_str()
            .unwrap()
            .to_lowercase()
            .contains("deepseek")),
        "{entries:?}"
    );

    let (_, out, _) = run_served(
        &db,
        &daemon,
        &["--json", "catalog", "list", "--tag", "official"],
    );
    let tagged: serde_json::Value = serde_json::from_str(&out).unwrap();
    let tagged = tagged["entries"].as_array().unwrap();
    assert_eq!(tagged.len(), 1);
    assert!(tagged.iter().all(|e| e["tag"] == "official"), "{tagged:?}");
}

/// `providers add` records which catalog entry its endpoint names — the link
/// that costs the provider's requests at its own published rate instead of at
/// whichever entry happens to sort first. Nobody on the command line can state
/// it, so it is inferred; the CLI is the path with no shelf to name one.
#[test]
fn providers_add_links_the_catalog_entry() {
    let (_dir, db) = temp_db();
    let daemon = support::serve(&db);
    {
        // Seeded through the store: the cache is the daemon's table, so its
        // accessor there is the only one — the `Aux` copies are gone
        // (`migrate.local.md` §10.14).
        let store = Store::open(&db).unwrap();
        store
            .save_hub_cache(SEED_CATALOG, "2026-09-07T00:00:00Z")
            .unwrap();
    }

    // The URL `docs/cli.md` has users type: the entry's path, not its host.
    let (code, _, err) = run_served(
        &db,
        &daemon,
        &[
            "providers",
            "add",
            "--name",
            "DeepSeek",
            "--endpoint",
            "https://api.deepseek.com/anthropic",
            "--key",
            "sk-test",
        ],
    );
    assert_eq!(code, 0, "{err}");

    let store = kiwanod::store::Store::open(&db).unwrap();
    let rows: Vec<(String, Option<String>)> = store
        .list_providers()
        .unwrap()
        .into_iter()
        .map(|p| (p.name, p.catalog_id))
        .collect();
    assert_eq!(
        rows,
        [("DeepSeek".to_string(), Some("deepseek".to_string()))]
    );
}

/// Two entries, carrying only the fields the wire format requires (`id`, `name`,
/// `tag`, `rating`, `billing`) plus an endpoint list — the app derives the rest.
const SEED_CATALOG: &str = r#"{"total":2,"entries":[
    {"id":"deepseek","name":"DeepSeek","tag":"official","rating":4.8,
     "billing":"payg","currency":"USD",
     "endpoints":[{"protocol":"openai","endpoint":"https://api.deepseek.com"}]},
    {"id":"openrouter","name":"OpenRouter","tag":"aggregate","rating":4.5,
     "billing":"payg","currency":"USD",
     "endpoints":[{"protocol":"openai","endpoint":"https://openrouter.ai/api/v1"}]}
]}"#;

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
///
/// The witness is `alerts` rather than a mutation: the reload note that used to
/// prove this is gone entirely — the daemon re-reads its own route table after a
/// write, so there is nothing for the client to say. What still has to hold is
/// the split itself, and `alerts` says it on stderr whether or not anything is
/// over budget.
#[test]
fn json_stdout_stays_parseable_while_notes_go_to_stderr() {
    let (_dir, db) = temp_db();
    let daemon = support::serve(&db);
    let (code, out, err) = run_served(&db, &daemon, &["--json", "alerts"]);
    assert_eq!(code, 0, "{err}");
    serde_json::from_str::<serde_json::Value>(&out)
        .unwrap_or_else(|e| panic!("stdout is not a single JSON document ({e}): {out}"));
    assert!(
        err.contains("no provider is over its allowance"),
        "the note belongs on stderr: {err}"
    );
}

// ── insights ────────────────────────────────────────────────────────────────

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

/// Seed one request-log row with the fields the insights rules read. The
/// generic `seed_log` above pins "now" and one shape; the rules need control
/// over time (retry windows), sessions (growth) and tokens (cache rate).
#[allow(clippy::too_many_arguments)]
fn seed_insight_row(
    db: &Path,
    ts_unix: i64,
    agent: Option<&str>,
    session: Option<&str>,
    status: i64,
    error_kind: Option<&str>,
    input: i64,
    cache_read: i64,
) -> i64 {
    let store = Store::open(db).unwrap();
    store
        .insert_request_log(&kiwanod::store::RequestLogNew {
            ts: kiwanod::store::rfc3339_from_unix(ts_unix),
            method: "POST".into(),
            path: "/v1/messages".into(),
            query: None,
            agent: agent.map(str::to_string),
            attribution: Some("key".into()),
            provider_id: Some("p1".into()),
            model: Some("m1".into()),
            status_code: status,
            error_kind: error_kind.map(str::to_string),
            error_message: None,
            session_id: session.map(str::to_string),
            is_streaming: false,
            input_tokens: input,
            output_tokens: 10,
            cache_read_tokens: cache_read,
            cache_creation_tokens: 0,
            reasoning_tokens: 0,
            usage_missing: false,
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
            cost_off_peak: None,
            request_notes: None,
        })
        .unwrap()
}

#[test]
fn insights_reports_a_retry_storm_with_evidence() {
    let (_dir, db) = temp_db();
    let daemon = support::serve(&db);
    // The shape the real log showed on 2026-09-09: one errored request, then
    // resends every few seconds inside the same minute.
    let base = now_unix() - 3600;
    for i in 0..4 {
        seed_insight_row(
            &db,
            base + i * 5,
            Some("codex"),
            None,
            500,
            Some("protocol_mismatch"),
            100,
            0,
        );
    }

    let (code, out, err) = run_served(&db, &daemon, &["insights", "--days", "7"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("Kiwano insights"), "{out}");
    assert!(out.contains("Scorecard"), "{out}");
    assert!(
        out.contains("4 requests · 0 sessions · 1 agents · 4 errors (100.0%)"),
        "{out}"
    );
    assert!(out.contains("3 retries in 15s"), "{out}");
    assert!(out.contains("protocol_mismatch"), "{out}");
    assert!(out.contains("evidence #1 #2 #3 #4"), "{out}");
    assert!(out.contains("never leave this machine"), "{out}");

    let (code, out, err) = run_served(&db, &daemon, &["--json", "insights", "--days", "7"]);
    assert_eq!(code, 0, "{err}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["totals"]["requests"], 4);
    assert_eq!(v["totals"]["errors"], 4);
    assert_eq!(v["days"], 7);
    assert_eq!(v["scorecard"][0]["agent"], "codex");
    assert_eq!(v["scorecard"][0]["retries"], 3);
    assert_eq!(v["findings"][0]["tag"], "retry");
    assert_eq!(
        v["findings"][0]["evidence"],
        serde_json::json!([1, 2, 3, 4])
    );
}

#[test]
fn insights_flags_a_session_that_never_compacted() {
    let (_dir, db) = temp_db();
    let daemon = support::serve(&db);
    let base = now_unix() - 3600;
    for i in 0..5 {
        seed_insight_row(
            &db,
            base + i * 120,
            Some("cline"),
            Some("s-77"),
            200,
            None,
            10_000 * (1 << i),
            0,
        );
    }

    let (code, out, err) = run_served(&db, &daemon, &["insights"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("Top sessions by context growth"), "{out}");
    assert!(out.contains("s-77"), "{out}");
    assert!(out.contains("16.0×"), "{out}");
    assert!(out.contains("never compacted"), "{out}");
    // The bloat finding names the ends of the session, not all five rows.
    assert!(out.contains("evidence #1 … #5"), "{out}");
}

#[test]
fn insights_honors_the_agent_filter_and_rejects_a_bad_window() {
    let (_dir, db) = temp_db();
    let daemon = support::serve(&db);
    let base = now_unix() - 3600;
    seed_insight_row(&db, base, Some("codex"), None, 200, None, 1000, 0);
    seed_insight_row(&db, base + 1, Some("claude"), None, 200, None, 1000, 500);

    let (code, out, err) = run_served(&db, &daemon, &["insights", "--agent", "claude"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("claude"), "{out}");
    assert!(!out.contains("codex"), "{out}");
    // claude read 500 of 1500 input-side tokens from cache.
    assert!(out.contains("33%"), "{out}");

    // An empty window says so instead of printing empty tables.
    let (code, out, err) = run_served(&db, &daemon, &["insights", "--agent", "nobody"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("(no requests in window)"), "{out}");

    let (code, _, err) = run_served(&db, &daemon, &["insights", "--days", "0"]);
    assert_eq!(code, 2);
    assert!(err.contains("--days"), "{err}");
}

/// The MCP server is an opt-in feature: with the flag off (the default) the
/// command refuses before ever touching stdin.
#[test]
fn mcp_is_gated_by_the_features_flag() {
    let (_dir, db) = temp_db();
    let daemon = support::serve(&db);
    let (code, _out, err) = run_served(&db, &daemon, &["mcp"]);
    assert_eq!(code, 2);
    assert!(err.contains("Features"), "{err}");
}

/// Rule injection is an opt-in feature: `apply` refuses with the flag off, and
/// answers honestly when the window's insights teach nothing.
#[test]
fn rules_apply_is_gated_and_honest_about_empty_windows() {
    let (_dir, db) = temp_db();
    let daemon = support::serve(&db);
    let home = tempfile::tempdir().unwrap();
    let home_arg = home.path().display().to_string();

    // Flag off (the default): refused before any file is touched.
    let (code, _out, err) = run_served(
        &db,
        &daemon,
        &["--home", &home_arg, "rules", "apply", "claude"],
    );
    assert_eq!(code, 2);
    assert!(err.contains("Features"), "{err}");
    assert!(!home.path().join(".claude/CLAUDE.md").exists());

    // Flag on, nothing in the log: no rules, and still no file.
    let (code, _out, err) = run_served(
        &db,
        &daemon,
        &["settings", "set", "--key", "feat_rule_injection=true"],
    );
    assert_eq!(code, 0, "{err}");
    let (code, out, err) = run_served(
        &db,
        &daemon,
        &["--home", &home_arg, "rules", "apply", "claude"],
    );
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("no rules"), "{out}");
    assert!(!home.path().join(".claude/CLAUDE.md").exists());

    let (code, out, err) = run_served(
        &db,
        &daemon,
        &["--home", &home_arg, "rules", "status", "claude"],
    );
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("no rules applied"), "{out}");
}

/// The cache experiment is an opt-in feature: gated off it refuses, and on it
/// prints its report skeleton even over an empty window.
#[test]
fn cache_experiment_is_gated_and_reports_empty_windows() {
    let (_dir, db) = temp_db();
    let daemon = support::serve(&db);
    let (code, _out, err) = run_served(&db, &daemon, &["cache-experiment"]);
    assert_eq!(code, 2);
    assert!(err.contains("Features"), "{err}");

    let (code, _out, err) = run_served(
        &db,
        &daemon,
        &["settings", "set", "--key", "feat_cache_experiment=true"],
    );
    assert_eq!(code, 0, "{err}");
    let (code, out, err) = run_served(&db, &daemon, &["cache-experiment", "--days", "7"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("pairs 0"), "{out}");
    assert!(out.contains("too little data"), "{out}");
}

// ── client keys ─────────────────────────────────────────────────────────────

/// Minting a key prints the secret once, and nothing after that does.
///
/// The asymmetry is the contract: `add` and `rotate` are the two operations that
/// create a credential, so they are the two that may show it. Every read answers
/// with a mask, which is what makes a `clients list` safe to paste into a chat.
#[test]
fn clients_add_list_rotate_roundtrips_and_only_shows_the_secret_once() {
    let (_dir, db) = temp_db();
    let daemon = support::serve(&db);

    let (code, out, err) = run_served(
        &db,
        &daemon,
        &[
            "--json",
            "clients",
            "add",
            "--agent",
            "claude",
            "--label",
            "office laptop",
            "--model",
            "claude-sonnet-4-5",
            "--window",
            "day:100:requests",
        ],
    );
    assert_eq!(code, 0, "{err}");
    let created: serde_json::Value = serde_json::from_str(&out).unwrap();
    let id = created["id"].as_str().expect("handle").to_string();
    let key = created["key"].as_str().expect("secret").to_string();
    assert!(key.starts_with("kw-ag-claude-"), "{key}");
    assert!(id.starts_with("ck-"), "{id}");

    let (code, out, err) = run_served(&db, &daemon, &["--json", "clients", "list"]);
    assert_eq!(code, 0, "{err}");
    let listed: Vec<serde_json::Value> = serde_json::from_str(&out).unwrap();
    assert_eq!(listed.len(), 1);
    // A key that has carried nothing is *not dated* rather than dated blankly:
    // "never used" and "used, then idle" are the two answers a revocation
    // decision turns on.
    assert!(listed[0].get("last_used_at").is_none(), "{}", listed[0]);
    let (_, text, _) = run_served(&db, &daemon, &["clients", "list"]);
    assert!(text.contains("LAST USED"), "{text}");
    assert!(text.contains("never"), "{text}");
    assert_eq!(listed[0]["id"], id.as_str());
    assert_eq!(listed[0]["agent"], "claude");
    assert_eq!(listed[0]["label"], "office laptop");
    assert_eq!(listed[0]["model_allow"][0], "claude-sonnet-4-5");
    assert_eq!(listed[0]["limits"][0]["period"], "day");
    // The read carries no secret, and the mask is not a prefix of nothing: it is
    // shorter than the key and elides its middle.
    let masked = listed[0]["masked"].as_str().unwrap();
    assert_ne!(masked, key);
    assert!(!masked.contains(&key[7..]), "{masked}");
    assert!(masked.contains('…'), "{masked}");

    // Rotating hands back a new secret under the same handle, and keeps what the
    // key was allowed to do.
    let (code, out, err) = run_served(&db, &daemon, &["--json", "clients", "rotate", &id]);
    assert_eq!(code, 0, "{err}");
    let rotated: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(rotated["id"], id.as_str());
    assert_ne!(rotated["key"].as_str().unwrap(), key);
    let (_, out, _) = run_served(&db, &daemon, &["--json", "clients", "show", &id]);
    let shown: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(shown["label"], "office laptop");
    assert_eq!(shown["limits"].as_array().unwrap().len(), 1);

    // Clearing a window set and a policy is how a key is un-restricted, and the
    // listing then says so rather than showing an empty cell.
    let (code, _, err) = run_served(&db, &daemon, &["clients", "limits", &id]);
    assert_eq!(code, 0, "{err}");
    let (code, _, err) = run_served(
        &db,
        &daemon,
        &["clients", "policy", &id, "--provider", "p-anything"],
    );
    assert_eq!(code, 0, "{err}");
    let (_, out, _) = run_served(&db, &daemon, &["--json", "clients", "show", &id]);
    let shown: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert!(shown["limits"].as_array().unwrap().is_empty());
    assert_eq!(shown["provider_allow"][0], "p-anything");
    // Omitting --model on that call cleared the model list, which is what a flag
    // that names a list means.
    assert!(shown["model_allow"].as_array().unwrap().is_empty());

    let (code, _, err) = run_served(&db, &daemon, &["clients", "remove", &id]);
    assert_eq!(code, 0, "{err}");
    let (_, out, _) = run_served(&db, &daemon, &["--json", "clients", "list"]);
    assert_eq!(out.trim(), "[]");
}

/// A window that means nothing is refused at the command line, because the store
/// would drop it and the user would believe a limit they do not have.
#[test]
fn a_client_key_window_of_zero_is_refused() {
    let (_dir, db) = temp_db();
    let daemon = support::serve(&db);
    let (code, _, err) = run_served(
        &db,
        &daemon,
        &[
            "clients",
            "add",
            "--agent",
            "claude",
            "--window",
            "day:0:requests",
        ],
    );
    assert_ne!(code, 0);
    assert!(err.contains("not a ceiling"), "{err}");

    let (code, _, err) = run_served(
        &db,
        &daemon,
        &["clients", "add", "--agent", "claude", "--window", "day"],
    );
    assert_ne!(code, 0);
    assert!(err.contains("PERIOD:LIMIT"), "{err}");
}

/// Client keys are served by the daemon, so a CLI without one says so.
#[test]
fn client_keys_need_the_daemon() {
    let (_dir, db) = temp_db();
    let (code, _, err) = run(&db, &["clients", "list"]);
    assert_ne!(code, 0);
    assert!(
        !err.is_empty(),
        "a missing daemon is an error, not an empty list"
    );
}

// ── the OpenAI wire ─────────────────────────────────────────────────────────

/// A wire is declared once and then visible in the listing — otherwise a user
/// who set it could not tell it apart from never having set it.
#[test]
fn the_openai_wire_is_declared_and_listed() {
    let (_dir, db) = temp_db();
    let daemon = support::serve(&db);

    let (code, out, err) = run_served(
        &db,
        &daemon,
        &[
            "--json",
            "providers",
            "add",
            "--name",
            "relay",
            "--endpoint",
            "https://relay.example.com/v1",
            "--protocol",
            "openai",
            "--openai-wire",
            "chat",
            "--bind",
            "codex",
        ],
    );
    assert_eq!(code, 0, "{err}");
    let created: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(created["openai_wire"], "chat");

    // The list carries it as a restriction on the protocol, so the common case
    // (`both`) stays unmarked.
    let (code, out, err) = run_served(&db, &daemon, &["providers", "list"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("openai·chat"), "{out}");

    let (code, out, err) = run_served(
        &db,
        &daemon,
        &[
            "--json",
            "providers",
            "edit",
            created["id"].as_str().unwrap(),
            "--openai-wire",
            "both",
        ],
    );
    assert_eq!(code, 0, "{err}");
    let edited: serde_json::Value = serde_json::from_str(&out).unwrap();
    // Absent *is* `both`: the field reports a restriction, and clearing it
    // clears the field. (`created["openai_wire"]` above works because it was
    // set; an index into a missing key would be null.)
    assert!(edited.get("openai_wire").is_none(), "{edited}");
    let (_, out, _) = run_served(&db, &daemon, &["providers", "list"]);
    assert!(!out.contains("openai·"), "both is not a restriction: {out}");
}

/// A wire on a provider that is not OpenAI, or a value that is not a wire, is
/// refused — a stored restriction nothing reads is one nobody can be told about.
#[test]
fn a_wire_that_means_nothing_is_refused() {
    let (_dir, db) = temp_db();
    let daemon = support::serve(&db);

    let (code, _, err) = run_served(
        &db,
        &daemon,
        &[
            "providers",
            "add",
            "--name",
            "claude-direct",
            "--endpoint",
            "https://api.anthropic.com",
            "--protocol",
            "anthropic",
            "--openai-wire",
            "chat",
        ],
    );
    assert_ne!(code, 0);
    assert!(err.contains("OpenAI wire"), "{err}");

    let (code, _, err) = run_served(
        &db,
        &daemon,
        &[
            "providers",
            "add",
            "--name",
            "relay",
            "--endpoint",
            "https://relay.example.com/v1",
            "--protocol",
            "openai",
            "--openai-wire",
            "chatty",
        ],
    );
    assert_ne!(code, 0);
    assert!(err.contains("expected chat"), "{err}");
}

// ── history ─────────────────────────────────────────────────────────────────

/// The whole chain once: a home with a transcript in it, the CLI reading it, the
/// daemon storing it, and the dashboard seeing the spend — while the *request*
/// count stays what the gateway actually routed.
///
/// This is the feature's promise in one test: a fresh install's dashboard is not
/// empty, because the agent's own files were read. It is also where the two
/// halves of "体感一样" are pinned: the money includes the imported history, the
/// request count does not, and the imported rows appear as their own bucket
/// rather than being folded into a provider's.
#[test]
fn history_import_fills_a_fresh_install_and_the_dashboard_shows_it() {
    let (_dir, db) = temp_db();
    let daemon = support::serve(&db);
    let home = tempfile::tempdir().unwrap();

    // A Claude Code transcript, written the way the agent writes one: the
    // directory is a slug of the project path, each line a record, and the token
    // counts live in `message.usage`.
    let project = home.path().join("work").join("acme-api");
    std::fs::create_dir_all(&project).unwrap();
    let dir = home
        .path()
        .join(".claude")
        .join("projects")
        .join("-home-me-work-acme-api");
    std::fs::create_dir_all(&dir).unwrap();
    let record = |uuid: &str, ts: &str, input: i64, output: i64| {
        serde_json::json!({
            "type": "assistant",
            "uuid": uuid,
            "timestamp": ts,
            "sessionId": "11111111-2222-3333-4444-555555555555",
            "cwd": project.to_string_lossy(),
            "gitBranch": "main",
            "message": {
                "model": "claude-sonnet-4-5",
                "usage": { "input_tokens": input, "output_tokens": output },
                "content": [{ "type": "tool_use", "name": "Bash" }],
            },
        })
    };
    let lines = [
        record("r1", "2026-01-02T10:00:00Z", 1_000, 200),
        record("r2", "2026-01-02T10:05:00Z", 500, 100),
    ];
    let mut body = String::new();
    for line in &lines {
        body.push_str(&line.to_string());
        body.push('\n');
    }
    std::fs::write(dir.join("session.jsonl"), body).unwrap();

    // `--dry-run` first: it must report what it found without writing anything.
    let home_arg = home.path().display().to_string();
    let (code, out, err) = run(
        &db,
        &[
            "--home",
            &home_arg,
            "--json",
            "history",
            "import",
            "--dry-run",
        ],
    );
    assert_eq!(code, 0, "{err}");
    let dry: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(dry["usage"].as_array().unwrap().len(), 2);
    // Nothing was written, so the daemon has no scan stamp and no sessions.
    let (code, out, _) = run_served(&db, &daemon, &["--json", "sessions"]);
    assert_eq!(code, 0);
    assert_eq!(out.trim(), "[]");

    // Now the real import.
    let (code, out, err) = run_served(
        &db,
        &daemon,
        &["--home", &home_arg, "--json", "history", "import"],
    );
    assert_eq!(code, 0, "{err}");
    let report: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(report["usage_rows"], 2);
    assert_eq!(report["sessions"], 1);
    assert_eq!(report["skipped_by_watermark"], 0, "nothing was metered yet");

    // The dashboard's spend includes the imported rows...
    let (code, out, err) = run_served(&db, &daemon, &["--json", "dashboard", "--window", "all"]);
    assert_eq!(code, 0, "{err}");
    let dashboard: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(
        dashboard["input_tokens"].as_i64().unwrap() + dashboard["output_tokens"].as_i64().unwrap(),
        1_800,
        "1000+200+500+100 tokens of history: {dashboard}"
    );
    // ...and the request count does not: that tile is what this gateway routed.
    assert_eq!(
        dashboard["requests"].as_i64().unwrap_or(0),
        0,
        "imported rows are not gateway-routed requests: {dashboard}"
    );

    // The sessions list has it, with the project label rather than a path.
    let (code, out, err) = run_served(&db, &daemon, &["--json", "sessions"]);
    assert_eq!(code, 0, "{err}");
    let sessions: Vec<serde_json::Value> = serde_json::from_str(&out).unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0]["agent"], "claude");
    assert_eq!(
        sessions[0]["source"], "imported",
        "no traffic ran through this daemon: {sessions:?}"
    );
    assert_eq!(
        sessions[0]["requests"], 0,
        "requests are the gateway's count"
    );
    assert_eq!(sessions[0]["turns"], 2);
    assert!(
        sessions[0]["tool_calls"][0]["name"] == "Bash",
        "the tools are a list of names and counts: {}",
        sessions[0]
    );
    let project = sessions[0]["project"].as_str().unwrap();
    assert_eq!(project, "acme-api", "the label, not the path");
    assert!(
        !project.contains('/'),
        "a path must not cross the interface"
    );

    // The scan is stamped **per agent**, and the list says so: a reader added to
    // a later build reads its own stamp and backfills, instead of being suppressed
    // by a flag that was set before it existed.
    let (_, text, _) = run_served(&db, &daemon, &["sessions", "--days", "400"]);
    let stamped = text
        .lines()
        .find(|l| l.starts_with("scanned:"))
        .expect("the stamps are listed");
    assert!(stamped.contains("claude"), "{stamped}");
    assert!(stamped.contains("codex"), "{stamped}");

    // A second run changes nothing: the rows are identified by their import keys.
    let (code, _, err) = run_served(
        &db,
        &daemon,
        &["--home", &home_arg, "--json", "history", "import"],
    );
    assert_eq!(code, 0, "{err}");
    let (_, out, _) = run_served(&db, &daemon, &["--json", "dashboard", "--window", "all"]);
    let again: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(
        again["input_tokens"].as_i64().unwrap() + again["output_tokens"].as_i64().unwrap(),
        1_800,
        "a re-import refreshes rather than doubles: {again}"
    );
}

/// A home with nothing to read is not an error: it says so and exits 0, because
/// "this machine has no transcripts" is a normal state, not a failure.
#[test]
fn history_import_on_an_empty_home_says_so() {
    let (_dir, db) = temp_db();
    let daemon = support::serve(&db);
    let home = tempfile::tempdir().unwrap();
    let home_arg = home.path().display().to_string();

    let (code, out, err) = run_served(&db, &daemon, &["--home", &home_arg, "history", "import"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("nothing to import"), "{out}");

    // And an agent this build cannot read is refused rather than ignored.
    let (code, _, err) = run_served(&db, &daemon, &["clients", "list", "--agent", "nope"]);
    assert_eq!(
        code, 0,
        "an unknown agent is an empty list, not an error: {err}"
    );
    let (code, _, err) = run_served(
        &db,
        &daemon,
        &["--home", &home_arg, "history", "import", "--agent", "nope"],
    );
    assert_ne!(code, 0);
    assert!(err.contains("no reader for"), "{err}");
}

/// Declaring a price for one model leaves the provider's other declarations
/// alone — the write is a snapshot, so this command has to fold rather than
/// replace.
#[test]
fn declaring_a_price_keeps_the_providers_other_declarations() {
    let (_dir, db) = temp_db();
    let daemon = support::serve(&db);
    let provider = add_provider(&db, "relay", &[]);

    for (model, input) in [("acme-large", "3"), ("acme-small", "0.5")] {
        let (code, _, err) = run_served(
            &db,
            &daemon,
            &[
                "providers",
                "price",
                &provider,
                "--model",
                model,
                "--input",
                input,
                "--output",
                "15",
                "--currency",
                "USD",
            ],
        );
        assert_eq!(code, 0, "{err}");
    }

    let (_, out, _) = run_served(&db, &daemon, &["--json", "providers", "list"]);
    let providers: Vec<serde_json::Value> = serde_json::from_str(&out).unwrap();
    let prices = &providers[0]["prices"];
    assert_eq!(prices["currency"], "USD");
    let models: Vec<&str> = prices["models"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["model_id"].as_str().unwrap())
        .collect();
    assert_eq!(
        models,
        vec!["acme-large", "acme-small"],
        "both, not just the last"
    );

    // Re-declaring one model updates it in place rather than adding a second row.
    let (code, _, err) = run_served(
        &db,
        &daemon,
        &[
            "providers",
            "price",
            &provider,
            "--model",
            "acme-small",
            "--input",
            "0.9",
            "--output",
            "9",
        ],
    );
    assert_eq!(code, 0, "{err}");
    let (_, out, _) = run_served(&db, &daemon, &["--json", "providers", "list"]);
    let providers: Vec<serde_json::Value> = serde_json::from_str(&out).unwrap();
    let models = providers[0]["prices"]["models"].as_array().unwrap();
    assert_eq!(models.len(), 2);
    assert_eq!(models[1]["input"], "0.9");

    // A different currency would re-denominate the first model's rates, so it is
    // refused rather than silently applied.
    let (code, _, err) = run_served(
        &db,
        &daemon,
        &[
            "providers",
            "price",
            &provider,
            "--model",
            "acme-third",
            "--input",
            "1",
            "--output",
            "2",
            "--currency",
            "CNY",
        ],
    );
    assert_ne!(code, 0);
    assert!(err.contains("declared prices are in USD"), "{err}");

    // And clearing drops one, leaving the other.
    let (code, _, err) = run_served(
        &db,
        &daemon,
        &[
            "providers",
            "price",
            &provider,
            "--model",
            "acme-large",
            "--clear",
        ],
    );
    assert_eq!(code, 0, "{err}");
    let (_, out, _) = run_served(&db, &daemon, &["--json", "providers", "list"]);
    let providers: Vec<serde_json::Value> = serde_json::from_str(&out).unwrap();
    let models = providers[0]["prices"]["models"].as_array().unwrap();
    assert_eq!(models.len(), 1);
    assert_eq!(models[0]["model_id"], "acme-small");
}

/// The loop the command exists for: a model the Hub cannot price is priced by a
/// declaration, and the imported history shows the money.
#[test]
fn a_declared_price_prices_an_imported_models_rows() {
    let (_dir, db) = temp_db();
    let daemon = support::serve(&db);
    let provider = add_provider(&db, "relay", &[]);
    let home = tempfile::tempdir().unwrap();

    // A transcript whose model name the Hub does not price.
    let dir = home
        .path()
        .join(".claude")
        .join("projects")
        .join("-home-me-work-acme-api");
    std::fs::create_dir_all(&dir).unwrap();
    let record = serde_json::json!({
        "type": "assistant",
        "uuid": "r1",
        "timestamp": "2026-01-02T10:00:00Z",
        "sessionId": "22222222-3333-4444-5555-666666666666",
        "cwd": home.path().to_string_lossy(),
        "message": {
            "model": "a-plan-name",
            "usage": { "input_tokens": 1_000_000, "output_tokens": 0 },
            "content": [],
        },
    });
    std::fs::write(dir.join("s.jsonl"), format!("{record}\n")).unwrap();

    let home_arg = home.path().display().to_string();
    let (code, out, err) = run_served(
        &db,
        &daemon,
        &["--home", &home_arg, "--json", "history", "import"],
    );
    assert_eq!(code, 0, "{err}");
    let first: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(first["unpriced"], 1);
    assert_eq!(first["unpriced_models"][0]["model"], "a-plan-name");

    // The report named it; now declare what it costs and import again.
    let (code, _, err) = run_served(
        &db,
        &daemon,
        &[
            "providers",
            "price",
            &provider,
            "--model",
            "a-plan-name",
            "--input",
            "2",
            "--output",
            "10",
            "--currency",
            "USD",
        ],
    );
    assert_eq!(code, 0, "{err}");
    let (code, out, err) = run_served(
        &db,
        &daemon,
        &["--home", &home_arg, "--json", "history", "import"],
    );
    assert_eq!(code, 0, "{err}");
    let second: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(second["priced"].as_i64(), Some(1));
    assert_eq!(second["unpriced"].as_i64(), Some(0));

    // A million input tokens at $2 per million, and it shows up as money the
    // dashboard includes — in the bucket for spend that never came through the
    // gateway, since that is what it is.
    let (_, out, _) = run_served(&db, &daemon, &["--json", "dashboard", "--window", "all"]);
    let dashboard: serde_json::Value = serde_json::from_str(&out).unwrap();
    let cost = dashboard["cost"].as_f64().unwrap();
    assert!(
        (cost - 2.0).abs() < 1e-6,
        "one million input at 2/million: {cost}"
    );
    let bucket = dashboard["by_provider"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == "imported-history")
        .expect("the imported bucket");
    assert!(
        (bucket["cost"].as_f64().unwrap() - 2.0).abs() < 1e-6,
        "{bucket}"
    );
}

/// What each key spent, in the money it was spent in — and the rows that could
/// not be priced are named rather than folded into a total that is quietly short.
#[test]
fn clients_list_reports_what_each_key_spent() {
    let (_dir, db) = temp_db();
    let daemon = support::serve(&db);
    let store = Store::open(&db).unwrap();
    let laptop = store
        .upsert_client_key("kw-ag-claude-laptop", "claude")
        .unwrap();
    let desktop = store
        .upsert_client_key("kw-ag-claude-desktop", "claude")
        .unwrap();
    store.set_client_key_label(&laptop, Some("laptop")).unwrap();
    store
        .set_client_key_label(&desktop, Some("desktop"))
        .unwrap();

    let row = |key_id: &str, cost: Option<f64>, currency: Option<&str>, tokens: i64| {
        kiwanod::store::UsageRecord {
            ts: kiwanod::store::now_rfc3339(),
            agent: "claude".into(),
            provider_id: Some("p1".into()),
            client_key_id: Some(key_id.into()),
            model: Some("claude-sonnet-4-5".into()),
            input_tokens: tokens,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            latency_ms: None,
            status: "ok".into(),
            cost,
            cost_currency: currency.map(str::to_string),
            cost_off_peak: None,
            project: None,
            session_id: None,
            import_key: None,
        }
    };
    // The laptop spent in two currencies and had one row nobody could price; the
    // desktop spent nothing at all.
    store
        .record_usage(&row(&laptop, Some(1.5), Some("USD"), 1_000))
        .unwrap();
    store
        .record_usage(&row(&laptop, Some(20.0), Some("CNY"), 2_000))
        .unwrap();
    store
        .record_usage(&row(&laptop, None, None, 3_000))
        .unwrap();

    let (code, out, err) = run_served(&db, &daemon, &["--json", "clients", "list"]);
    assert_eq!(code, 0, "{err}");
    let keys: Vec<serde_json::Value> = serde_json::from_str(&out).unwrap();
    let by_name: std::collections::HashMap<&str, &serde_json::Value> = keys
        .iter()
        .map(|k| (k["label"].as_str().unwrap_or(""), k))
        .collect();
    let laptop_json = by_name["laptop"];
    assert_eq!(laptop_json["spend"]["requests"], 3);
    assert_eq!(laptop_json["spend"]["tokens"], 6_000);
    assert_eq!(
        laptop_json["spend"]["unpriced_rows"], 1,
        "the row nobody could price is counted and named, not hidden"
    );
    let cost = laptop_json["spend"]["cost"].as_array().unwrap();
    assert_eq!(cost.len(), 2, "two currencies, kept apart");
    assert_eq!(cost[0]["currency"], "CNY");
    assert_eq!(cost[0]["amount"], 20.0);
    assert_eq!(cost[1]["currency"], "USD");
    assert_eq!(cost[1]["amount"], 1.5);

    // The text rendering says the same thing, and folds the shortfall into a
    // parenthetical rather than into the amount.
    let (_, text, _) = run_served(&db, &daemon, &["clients", "list", "--days", "7"]);
    assert!(
        text.contains("SPENT 7D"),
        "the header names its window: {text}"
    );
    assert!(
        text.contains("20.00 CNY + 1.50 USD (+1 unpriced)"),
        "{text}"
    );
    // A key that spent nothing shows the same `-` its other empty cells do,
    // rather than a zero that would read as "spent nothing, and that is a figure".
    let desktop_row = text
        .lines()
        .find(|l| l.contains("desktop"))
        .expect("the desktop's row");
    assert!(desktop_row.contains("| -  "), "{desktop_row}");
}
