//! A daemon for the CLI's integration tests to talk to.
//!
//! The CLI is becoming a client of the daemon's admin API (`migrate.local.md`
//! §14.1), which means the tests that drive the command tree in-process need
//! something on the other end. This is it: the **real** admin router
//! ([`admin_plane_router`]) served on a real endpoint, from the same
//! `GatewayState` the daemon builds at startup.
//!
//! # Why it is written here rather than shared
//!
//! `crates/gateway/src/server/admin_ipc.rs` has a `test_support` module doing
//! almost this, but it is `cfg(test) + pub(crate)` — sealed inside the gateway
//! crate and invisible to every other crate's integration tests. Lifting it into
//! the daemon's public surface would ship test scaffolding in the released
//! binary (or need a feature flag whose unification reaches the normal build) to
//! save about thirty lines. The duplication is the cheaper side of that trade,
//! and it is test-only code either way.
//!
//! # What it deliberately does not do
//!
//! **It does not touch the environment.** `KIWANO_DAEMON_ADDR` and
//! `KIWANO_ADMIN_SOCKET` are process-global, and the tests in a binary run in
//! parallel threads — so a test that set one would be pointing its neighbours at
//! its own daemon. The endpoint is handed over per invocation instead, through
//! the CLI's most specific flag, `--admin-socket`.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use kiwanod::server::{admin_plane_router, AdminEndpoint, GatewayState};
use kiwanod::store::Store;

/// A daemon serving the admin plane for one test's database.
pub struct TestDaemon {
    endpoint: AdminEndpoint,
}

impl TestDaemon {
    /// The endpoint as the CLI takes it: what `--admin-socket` wants.
    pub fn socket(&self) -> String {
        self.endpoint.describe()
    }
}

/// Serve `db`'s admin plane beside it, returning once the endpoint answers.
///
/// `GatewayState::new` mints the admin token into `db` on the way up
/// (`crates/gateway/src/server/mod.rs:228`), which is what makes the CLI work
/// here without being told anything: it reads the token from the database it
/// shares with the daemon.
pub fn serve(db: &Path) -> TestDaemon {
    let store = Store::open(db).expect("open the test database");
    let state = Arc::new(GatewayState::new(store).expect("build the gateway state"));
    let endpoint = test_endpoint(db);
    let app = admin_plane_router(state);
    let bound = endpoint.clone();
    // The thread is deliberately never joined: `axum::serve` runs until the
    // process exits, which is exactly what a test wants from it — the same
    // shape the gateway's own `test_support` uses.
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a test runtime");
        runtime.block_on(async move {
            let listener = bound.bind().await.expect("bind the admin endpoint");
            let _ = axum::serve(listener, app).await;
        });
    });
    assert!(
        wait_until_answering(&endpoint),
        "the admin plane never came up on {}",
        endpoint.describe()
    );
    TestDaemon { endpoint }
}

/// An endpoint that is unique to one test, and that `run` will *not* find.
///
/// Deliberately not `AdminEndpoint::beside_db`: that is the path the CLI
/// resolves on its own, so a daemon served there would also be reached by `run`
/// — and `run` has to keep meaning "no daemon", because that is a state the CLI
/// must handle and three tests assert it (decision D3). Naming the endpoint
/// something else keeps the two apart on every platform, and leaves
/// `--admin-socket` as the one way a test says which daemon it means.
///
/// The name is unique per test either way: a socket inside the test's own
/// temporary directory on unix, and there a pipe named after that directory —
/// `beside_db` would derive it from the *user* and every test in the process
/// would share one.
fn test_endpoint(db: &Path) -> AdminEndpoint {
    let dir = db.parent().expect("the database has a directory");
    #[cfg(unix)]
    {
        AdminEndpoint::parse(&dir.join("kiwano-test.sock").display().to_string())
    }
    #[cfg(windows)]
    {
        let unique = dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "endpoint".to_string());
        AdminEndpoint::parse(&format!("kiwano-test-{unique}"))
    }
}

/// Poll `endpoint` until a connect succeeds.
fn wait_until_answering(endpoint: &AdminEndpoint) -> bool {
    for _ in 0..200 {
        if endpoint.connect(Duration::from_millis(50)).is_ok() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}
