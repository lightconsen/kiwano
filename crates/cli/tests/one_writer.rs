//! The other half of the rule `app/src-tauri/tests/one_writer.rs` enforces: the
//! CLI does not open the daemon's database either.
//!
//! That file was written first and scans **one** client — `MANIFEST_DIR/src`,
//! the app's sources — so "one writer" held for the app and was merely true by
//! accident for the CLI (`migrate.local.md` §14.1's D1 says as much, and
//! `migrate.local.md` §10.48 records what closing it took). The CLI stopped
//! opening the shared database; nothing made that a property rather than a
//! state of affairs.
//!
//! There is one database the CLI *does* open, and it is the client's own —
//! `local.db` beside the daemon's, holding this machine's facts. So the rule is
//! not "no database": it is **no handle on the daemon's**.

use std::path::{Path, PathBuf};

/// The sources a command is written in.
fn cli_sources() -> Vec<(PathBuf, String)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut out = Vec::new();
    let mut stack = vec![dir.clone()];
    while let Some(next) = stack.pop() {
        for entry in std::fs::read_dir(&next).expect("a readable source directory") {
            let path = entry.expect("a directory entry").path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push((
                    path.clone(),
                    std::fs::read_to_string(&path).expect("a readable source file"),
                ));
            }
        }
    }
    assert!(
        !out.is_empty(),
        "no sources found under {} — this test would pass vacuously",
        dir.display()
    );
    out
}

/// Lines that are not comments, so a sentence about the store is not a use of it.
fn code_lines(source: &str) -> impl Iterator<Item = &str> {
    source
        .lines()
        .map(str::trim_start)
        .filter(|l| !l.starts_with("//"))
}

/// The gateway's store is never named, so a handle on it cannot exist.
///
/// Stronger than "no writes": a client with no way to *name* the type cannot
/// read the daemon's rows either, and reading is how the last few of them were
/// found (`migrate.local.md` §10.44). It is also the assertion that fails on the
/// change that would undo the work — one `use kiwanod::store::Store` away.
#[test]
fn the_cli_cannot_name_the_daemons_store() {
    let mut violations = Vec::new();
    for (path, text) in cli_sources() {
        for (n, line) in code_lines(&text).enumerate() {
            for needle in [
                "kiwanod::store::Store",
                "Store::open",
                "Store::open_in_memory",
            ] {
                if line.contains(needle) {
                    violations.push(format!("{}:{}: {needle}", path.display(), n + 1));
                }
            }
        }
    }
    assert!(
        violations.is_empty(),
        "the CLI reached for the daemon's database handle. Every read and write \
         goes over the admin plane now, and what this machine keeps for itself is \
         `local.db` behind `Aux` (`migrate.local.md` §14.1 D1, §10.48). Found:\n  {}",
        violations.join("\n  ")
    );
}

/// The one database it does open is its own, and it is opened beside the
/// daemon's rather than instead of it.
///
/// The positive half of the rule above: "no store" would also be satisfied by a
/// CLI that scraped by with no local state at all, and that is not the shape —
/// declared agent directories, injected-rule marks and takeover backups live
/// here, and they have to survive a daemon on another machine.
#[test]
fn the_one_database_it_opens_is_the_clients_own() {
    let sources = cli_sources();
    let lib = sources
        .iter()
        .find(|(p, _)| p.file_name().is_some_and(|n| n == "lib.rs"))
        .map(|(_, t)| t)
        .expect("the crate root");

    assert!(
        lib.contains("Aux::open(&self.local_db)"),
        "the auxiliary connection is opened on the client's own file; if this \
         moved, read the comment above it before changing this test"
    );
    assert!(
        lib.contains("local_db_path(&db.path)") || lib.contains("paths::local_db_path"),
        "and that file is resolved *from* the database path, so moving one moves \
         the other (`migrate.local.md` §10.41)"
    );
}
