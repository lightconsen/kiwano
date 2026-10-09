//! The app does not write the shared database.
//!
//! # Why this is a test and not a compiler error
//!
//! `migrate.local.md` §10 (1) wanted the guarantee the compiler gives: drop the
//! app's `kiwanod` dependency and every missed `store.…` call stops compiling.
//! §5 #5 makes that impossible — the aggregation layer stays on the client (it
//! reads facts about *this machine*), and that layer takes a `&Store`, so the app
//! must be able to name `Store`.
//!
//! §10.21 records the resolution: the property that matters is not "the app
//! cannot name a store" but **"the app cannot write one"**. That is what this
//! file checks, and it is checkable in a way the compiler cannot manage — a
//! `&Store` allows reads and writes alike, so no signature distinguishes them.
//!
//! # What it catches, and what it cannot
//!
//! It catches the two ways a write gets in by hand: calling a mutating method on
//! the handle, and opening a database of one's own. It does **not** catch
//! passing the handle to a function that writes — that needs the rows refactor
//! §10.21 describes, and until then this file's second half is the honest
//! statement of the gap rather than a claim that it is closed.
//!
//! The method list is an **allowlist**: a method nobody thought of fails the
//! test rather than passing it, which is the direction that catches new code.

use std::path::{Path, PathBuf};

/// Every method the app may call on its store or auxiliary handle.
///
/// Reads only, and deliberately short — it is the whole surface the app is
/// allowed to touch, so growing it is a decision rather than an accident.
const READ_ONLY_METHODS: [&str; 2] = ["manual_agent_dirs", "gateway_setting"];

fn app_sources() -> Vec<(PathBuf, String)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&dir).expect("the app's src directory") {
        let path = entry.expect("a directory entry").path();
        if path.extension().is_some_and(|e| e == "rs") {
            let text = std::fs::read_to_string(&path).expect("a readable source file");
            out.push((path, text));
        }
    }
    assert!(
        !out.is_empty(),
        "no sources found under {} — this test would pass vacuously",
        dir.display()
    );
    out
}

/// The method a call names, when the receiver is one of the app's two handles.
///
/// Matches `state.store.<name>(` and `state.aux.<name>(` — the only way the app
/// reaches either one, since both live in `AppState`.
fn handle_calls(source: &str) -> Vec<String> {
    let mut found = Vec::new();
    for line in source.lines() {
        let line = line.trim_start();
        // A commented-out call is not a call.
        if line.starts_with("//") {
            continue;
        }
        for handle in ["state.store.", "state.aux."] {
            if let Some(rest) = line.split(handle).nth(1) {
                let name: String = rest
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                if !name.is_empty() {
                    found.push(name);
                }
            }
        }
    }
    found
}

#[test]
fn the_app_calls_only_reads_on_its_database_handles() {
    let mut violations = Vec::new();
    for (path, text) in app_sources() {
        for name in handle_calls(&text) {
            if !READ_ONLY_METHODS.contains(&name.as_str()) {
                violations.push(format!("{}: state.store.{name}(…)", path.display()));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "the app wrote to the shared database — every write belongs to the \
         daemon, which is what makes it the one writer (`migrate.local.md` \
         §10.21). Found:\n  {}",
        violations.join("\n  ")
    );
}

/// A handle of one's own is the other way in — with one legitimate exception,
/// and naming it is the point.
///
/// `run` opens both connections once, at startup, and hands them to `AppState`;
/// every other module takes them from there. So `lib.rs` may open, and anything
/// else that does is either reading a different file than the daemon's or about
/// to write this one.
#[test]
fn the_app_does_not_open_a_database_of_its_own() {
    let mut violations = Vec::new();
    for (path, text) in app_sources() {
        if path.file_name().is_some_and(|n| n == "lib.rs") {
            continue;
        }
        for opener in ["Store::open", "Aux::open"] {
            if text.contains(opener) {
                violations.push(format!("{}: {opener}", path.display()));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "the app opened a database handle of its own; only `run` does, and it \
         hands the two connections to `AppState`. Found:\n  {}",
        violations.join("\n  ")
    );
}

/// The gap, stated rather than implied: the app *passes* its store to these
/// functions, and only a signature change can say whether one of them writes.
///
/// Pinning the list is the next best thing — a new pass-through shows up here,
/// where it can be looked at, instead of appearing silently in a diff.
#[test]
fn the_functions_the_app_hands_its_store_to_are_the_known_ones() {
    const KNOWN: [&str; 10] = [
        "vm::build_dashboard",
        "vm::build_footer_stats",
        "vm::build_provider_vms",
        "vm::build_settings",
        "vm::check_usage_alerts",
        "vm::set_agent_takeover",
        "vm::update_provider",
        "vm::export_request_logs_csv",
        "pricing::currency_meta",
        // Reads the settings blob — the row the daemon patches — for the tray's
        // close behaviour. Added when the test caught it, which is the test
        // doing its job rather than a reason to loosen it.
        "vm::ui_settings",
    ];
    let mut found: Vec<String> = Vec::new();
    for (_, text) in app_sources() {
        for line in text.lines() {
            let line = line.trim_start();
            if line.starts_with("//") || !line.contains("&state.store") {
                continue;
            }
            // The callee is the token right before the argument, so a binding
            // (`let vms = vm::build_provider_vms(&state.store…`) reads as the
            // function rather than as the binding.
            if let Some((before, _)) = line.split_once("(&state.store") {
                if let Some(callee) = before.split_whitespace().last() {
                    found.push(callee.trim_end_matches('(').to_string());
                }
            }
        }
    }
    found.sort();
    found.dedup();
    let unknown: Vec<&String> = found
        .iter()
        .filter(|c| !KNOWN.contains(&c.as_str()))
        .collect();
    assert!(
        unknown.is_empty(),
        "the app hands its store to a function this test does not know. If the \
         callee only reads, add it to KNOWN; if it writes, the write belongs to \
         the daemon (`migrate.local.md` §10.21). Found:\n  {}",
        unknown
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join("\n  ")
    );
    assert!(
        !found.is_empty(),
        "no store pass-throughs found at all — either the app stopped reading \
         the database (then delete this test) or the scan is broken"
    );
}

/// The third thing this file is for: a command that talks to the daemon must not
/// do it on the main thread.
///
/// `DaemonApi`'s transport is a blocking socket — that is deliberate, and the
/// client methods say so — so a plain `#[tauri::command]` runs the whole round
/// trip on the UI thread. It is invisible while the daemon answers from SQLite
/// and obvious the moment it does not: `sync_hub`, `get_plan_quota` and the
/// probes all reach the network on the far side, and a frozen window during a
/// sync is the symptom.
///
/// `#[tauri::command(async)]` is Tauri's "not the main thread" (it spawns the
/// call on a worker), and it is the right annotation for a *synchronous* body
/// that blocks. An `async fn` would be the wrong one: it blocks a runtime worker
/// while looking like it yields.
#[test]
fn every_command_that_reaches_the_daemon_is_off_the_main_thread() {
    let mut violations = Vec::new();
    for (path, text) in app_sources() {
        let lines: Vec<&str> = text.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            if !line.contains("DaemonApi::connect()") {
                continue;
            }
            // The annotation above the function this call sits in — found by
            // walking back to the function's own opening line first, since a
            // body can be twenty lines long and a fixed look-back would miss
            // the annotation entirely.
            let enclosing_fn = (0..=i).rev().find(|&j| {
                let l = lines[j].trim_start();
                l.starts_with("pub fn ") || (l.starts_with("fn ") && !l.starts_with("fn main"))
            });
            let found = enclosing_fn.and_then(|f| {
                (0..f).rev().take(8).find_map(|j| {
                    (lines[j].contains("#[tauri::command")
                        || !lines[j].trim_start().starts_with("#["))
                    .then(|| lines[j].trim().to_string())
                })
            });
            // The other legitimate escape: a call handed to `spawn_blocking`,
            // which is off the main thread whatever encloses it — the startup
            // sync is a spawned task rather than a command.
            let handed_to_a_worker = lines[..=i]
                .iter()
                .rev()
                .take(12)
                .any(|l| l.contains("spawn_blocking"));
            match found {
                _ if handed_to_a_worker => {}
                Some(attr) if attr.contains("async") => {}
                Some(attr) => violations.push(format!(
                    "{}:{} — `{attr}` runs on the main thread",
                    path.display(),
                    i + 1
                )),
                None => violations.push(format!(
                    "{}:{} — a daemon call outside a command and outside \
                     `spawn_blocking`?",
                    path.display(),
                    i + 1
                )),
            }
        }
    }
    assert!(
        violations.is_empty(),
        "these commands make a blocking daemon round trip on the main thread \
         (`migrate.local.md` §10.26). Annotate them `#[tauri::command(async)]`:\n  {}",
        violations.join("\n  ")
    );
}
