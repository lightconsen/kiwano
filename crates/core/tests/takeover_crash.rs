//! Crash consistency for `set_agent_takeover`.
//!
//! A takeover commits two halves that cannot commit together: rows in the
//! shared store, and the agent's own config files on disk. Between them the
//! process can die, and each window leaves a different residue. This file
//! produces those windows the only way that means anything — a **child process
//! that dies without unwinding** — and then runs [`reconcile_takeovers`] over
//! what it left, asserting the residue converges.
//!
//! Why a child process: an in-process test that stops early still unwinds, and
//! `enable`'s own rollback then cleans up. That is the *opposite* of the case
//! under study, and a test that lets it happen proves nothing. `crash_worker`
//! below is selected by name in a child of this same test binary (see
//! `crash_at`), and it ends with `std::process::abort` — no destructors, no
//! rollback, no second chance.
//!
//! The windows, and what each one should converge to:
//!
//! | died | before converge | after |
//! |---|---|---|
//! | after the store half | key registered, config untouched | key revoked |
//! | mid file half | config partly rewritten | config back, key revoked |
//! | before the applied mark | config written, row still pending | row applied |
//!
//! [`reconcile_takeovers`]: kiwano_core::vm::takeover::reconcile_takeovers

use std::path::{Path, PathBuf};
use std::process::Command;

use kiwano_core::detect::ShellVars;
use kiwano_core::takeover::{enable, live_placeholder_key};
use kiwano_core::vm::takeover::{
    mark_applied, phase_state, reconcile_takeovers, StateHalf, TakeoverFinding,
};
use kiwano_core::vm::Aux;
use kiwanod::store::Store;

/// The agent every window uses unless the window needs more than one file.
/// `claude` is the simplest built-in: one config file, no discovery.
const AGENT: &str = "claude";
/// A built-in whose takeover writes **three** files, so a death after the first
/// one leaves a config that is genuinely half-rewritten. `claude` cannot show
/// that window — one file is all of it.
const MULTI_FILE_AGENT: &str = "commandcode";

const DATA_PORT: u16 = 8317;

const ENV_HOME: &str = "KIWANO_CRASH_HOME";
const ENV_DB: &str = "KIWANO_CRASH_DB";
const ENV_AGENT: &str = "KIWANO_CRASH_AGENT";
const ENV_AT: &str = "KIWANO_CRASH_AT";

/// The fixture: a home with one agent config already in it, and the paths the
/// child will be handed through the environment.
struct Fixture {
    _tmp: tempfile::TempDir,
    home: PathBuf,
    db: PathBuf,
    agent: &'static str,
    config: PathBuf,
}

impl Fixture {
    fn new(agent: &'static str) -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        let db = home.join("kiwano.db");
        let config = agent_config(agent, &home);
        if let Some(parent) = config.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        // Only where the takeover refuses a missing file: `claude` must have a
        // config to read (`read_originals`), the additive agents may not.
        if agent == AGENT {
            std::fs::write(&config, "{\n  \"theme\": \"dark\"\n}\n").unwrap();
        }
        Self {
            _tmp: tmp,
            home,
            db,
            agent,
            config,
        }
    }

    fn store(&self) -> Store {
        Store::open(&self.db).unwrap()
    }

    fn aux(&self) -> Aux {
        Aux::open(&self.db).unwrap()
    }

    fn keys_for(&self, store: &Store, agent: &str) -> Vec<String> {
        store
            .list_placeholder_keys()
            .unwrap()
            .into_iter()
            .filter(|k| k.agent == agent)
            .map(|k| k.key)
            .collect()
    }
}

/// Where an agent keeps the config this test watches. Kept as a literal rather
/// than derived from `takeover_paths`, because a test that asks the code under
/// test where to look cannot notice the code looking somewhere else.
fn agent_config(agent: &str, home: &Path) -> PathBuf {
    match agent {
        "commandcode" => home.join(".commandcode").join("settings.json"),
        _ => home.join(".claude").join("settings.json"),
    }
}

/// Run `crash_worker` in a child of this test binary and return when it has
/// died. The child takes the same fixture paths, so what it leaves is what this
/// process then inspects.
fn crash_at(fixture: &Fixture, at: &str) {
    let exe = std::env::current_exe().expect("the test binary's own path");
    let status = Command::new(exe)
        // One test, by name: the child runs the worker, not the suite.
        .args(["--exact", "crash_worker", "--nocapture"])
        .env(ENV_HOME, &fixture.home)
        .env(ENV_DB, &fixture.db)
        .env(ENV_AGENT, fixture.agent)
        .env(ENV_AT, at)
        .status()
        .expect("spawn the crashing child");
    assert!(
        !status.success(),
        "the child was supposed to die at `{at}` and returned {status:?} instead — \
         without a real death this test measures nothing"
    );
}

/// The child. A no-op in the parent run (no `KIWANO_CRASH_AT` in the
/// environment), and the whole point in the child.
#[test]
fn crash_worker() {
    let Ok(at) = std::env::var(ENV_AT) else {
        return;
    };
    let home = PathBuf::from(std::env::var(ENV_HOME).expect(ENV_HOME));
    let db = PathBuf::from(std::env::var(ENV_DB).expect(ENV_DB));
    let agent = std::env::var(ENV_AGENT).expect(ENV_AGENT);
    let store = Store::open(&db).expect("child: store");
    let aux = Aux::open(&db).expect("child: aux");
    let vars = ShellVars::new();

    match at.as_str() {
        "after_state" => {
            phase_state(&store, &aux, &agent, &home, None, StateHalf::InProcess)
                .expect("child: phase_state");
            std::process::abort();
        }
        "mid_files" => {
            let prepared = phase_state(&store, &aux, &agent, &home, None, StateHalf::InProcess)
                .expect("child: phase_state");
            // Die after the first file of the file half: the config is left
            // half-rewritten, which is the state the backup exists for.
            let _ = kiwano_core::takeover::enable_with(
                &aux,
                &agent,
                &prepared.key,
                DATA_PORT,
                &home,
                &vars,
                |index| {
                    if index == 0 {
                        std::process::abort();
                    }
                },
            );
            std::process::abort();
        }
        "before_mark" => {
            let prepared = phase_state(&store, &aux, &agent, &home, None, StateHalf::InProcess)
                .expect("child: phase_state");
            enable(&aux, &agent, &prepared.key, DATA_PORT, &home, &vars).expect("child: enable");
            // Every file is written; only the mark is missing.
            std::process::abort();
        }
        other => panic!("unknown crash point `{other}`"),
    }
}

#[test]
fn crash_after_the_store_half_converges_by_revoking_the_key() {
    let f = Fixture::new(AGENT);
    crash_at(&f, "after_state");

    let store = f.store();
    let aux = f.aux();
    let vars = ShellVars::new();

    // What the child left: a key with no config carrying it.
    assert_eq!(f.keys_for(&store, AGENT).len(), 1, "the store half landed");
    assert!(
        live_placeholder_key(AGENT, &f.home, &vars).is_none(),
        "the file half did not"
    );

    let findings = reconcile_takeovers(&store, &aux, &f.home, &vars).unwrap();
    assert_eq!(findings.len(), 1, "got {findings:?}");
    match &findings[0] {
        TakeoverFinding::StoreWithoutConfig { agent, op_id } => {
            assert_eq!(agent, AGENT);
            assert!(!op_id.is_empty(), "the finding names the operation");
        }
        other => panic!("expected StoreWithoutConfig, got {other:?}"),
    }
    assert!(f.keys_for(&store, AGENT).is_empty(), "the key is revoked");
    assert!(aux.load_takeover_op(AGENT).is_none(), "the row is cleared");

    // And it stays converged: a second pass has nothing to do.
    assert!(reconcile_takeovers(&store, &aux, &f.home, &vars)
        .unwrap()
        .is_empty());
}

#[test]
fn crash_mid_file_half_puts_the_config_back_and_revokes() {
    let f = Fixture::new(MULTI_FILE_AGENT);
    crash_at(&f, "mid_files");

    let store = f.store();
    let aux = f.aux();
    let vars = ShellVars::new();

    // The child wrote the first of three files and died. The evidence is the
    // file itself: the fixture created none for this agent, so its existence
    // *is* the partial write.
    assert!(
        std::fs::read_to_string(&f.config).is_ok(),
        "the first of the three files should have been written before the death"
    );
    assert_eq!(f.keys_for(&store, MULTI_FILE_AGENT).len(), 1);

    let findings = reconcile_takeovers(&store, &aux, &f.home, &vars).unwrap();
    assert_eq!(findings.len(), 1, "got {findings:?}");
    assert!(matches!(
        findings[0],
        TakeoverFinding::StoreWithoutConfig { .. }
    ));

    // The file half is taken back: what the fixture wrote (nothing, for this
    // agent) is what is there now, and no key is registered for it.
    assert_eq!(
        std::fs::read_to_string(&f.config).ok(),
        None,
        "the file the takeover created is removed again"
    );
    assert!(f.keys_for(&store, MULTI_FILE_AGENT).is_empty());
    assert!(aux.load_takeover_op(MULTI_FILE_AGENT).is_none());
}

#[test]
fn crash_before_the_mark_converges_by_marking() {
    let f = Fixture::new(AGENT);
    crash_at(&f, "before_mark");

    let store = f.store();
    let aux = f.aux();
    let vars = ShellVars::new();

    // Both halves landed; only the mark is missing.
    assert_eq!(f.keys_for(&store, AGENT).len(), 1);
    assert_eq!(
        live_placeholder_key(AGENT, &f.home, &vars).as_deref(),
        f.keys_for(&store, AGENT).first().map(String::as_str),
        "the config carries our key"
    );
    assert!(aux
        .load_takeover_op(AGENT)
        .is_some_and(|op| !op.is_applied()));

    let findings = reconcile_takeovers(&store, &aux, &f.home, &vars).unwrap();
    assert_eq!(findings.len(), 1, "got {findings:?}");
    assert!(matches!(
        findings[0],
        TakeoverFinding::ConfigWithoutMark { .. }
    ));

    // The takeover is kept — it did land — and the row now says so.
    assert!(aux
        .load_takeover_op(AGENT)
        .is_some_and(|op| op.is_applied()));
    assert_eq!(f.keys_for(&store, AGENT).len(), 1, "the key stays");
    assert!(live_placeholder_key(AGENT, &f.home, &vars).is_some());
}

#[test]
fn a_replayed_operation_does_not_mint_a_second_key() {
    let f = Fixture::new(AGENT);
    let store = f.store();
    let aux = f.aux();

    let first = phase_state(&store, &aux, AGENT, &f.home, None, StateHalf::InProcess).unwrap();
    // The same operation, delivered again — a retry, not a new takeover.
    let again = phase_state(
        &store,
        &aux,
        AGENT,
        &f.home,
        Some(&first.op_id),
        StateHalf::InProcess,
    )
    .unwrap();

    assert_eq!(again.key, first.key, "the replay reuses the key it minted");
    assert_eq!(again.op_id, first.op_id);
    assert_eq!(
        f.keys_for(&store, AGENT),
        vec![first.key.clone()],
        "one key, not two"
    );

    // And a completed operation can be marked exactly once.
    mark_applied(&aux, AGENT, &first.op_id).unwrap();
    assert!(aux.load_takeover_op(AGENT).unwrap().is_applied());
}

#[test]
fn reconcile_leaves_agents_without_an_operation_row_alone() {
    // Every install that predates this mechanism has keys and backups and no
    // operation row. Those are not torn takeovers, and reporting them would
    // invent a problem out of a missing record.
    let f = Fixture::new(AGENT);
    let store = f.store();
    let aux = f.aux();
    store
        .upsert_placeholder_key("kw-ag-claude-legacy", AGENT)
        .unwrap();

    let findings = reconcile_takeovers(&store, &aux, &f.home, &ShellVars::new()).unwrap();
    assert!(findings.is_empty(), "got {findings:?}");
    assert_eq!(
        f.keys_for(&store, AGENT),
        vec!["kw-ag-claude-legacy".to_string()],
        "the pre-existing key is untouched"
    );
}
