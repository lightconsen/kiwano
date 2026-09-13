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

use kiwano_gateway::store::Store;

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
