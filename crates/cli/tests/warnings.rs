//! The warnings a command emits while it is changing something.
//!
//! Driven through the real binary rather than `run_with`, because that is the
//! only way to observe the stream in question: the library writes to the writers
//! it is handed, while warnings go to the process's stderr through the logger
//! `main` installs. The rest of the command tree is driven in-process (see
//! `cli.rs`); this one file pays for a process to prove the wiring.

use std::process::Command;

/// A temporary home holding an `opencode.json` whose `provider` is not an
/// object — which the takeover normalizes (replacing the field) and says so.
fn home_with_unnormalizable_opencode() -> tempfile::TempDir {
    let home = tempfile::tempdir().unwrap();
    let dir = home.path().join(".config").join("opencode");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("opencode.json"), r#"{"provider":"nope"}"#).unwrap();
    home
}

fn run_takeover(home: &std::path::Path, quiet: bool) -> (i32, String) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_kiwano"));
    cmd.env("HOME", home)
        .env("KIWANO_DB_PATH", home.join("kiwano.db"))
        .args(["agents", "takeover", "opencode"]);
    if quiet {
        cmd.arg("--quiet");
    }
    let out = cmd.output().expect("the kiwano binary runs");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn a_normalization_the_user_should_know_about_reaches_stderr() {
    let home = home_with_unnormalizable_opencode();
    let (code, stderr) = run_takeover(home.path(), false);

    assert_eq!(code, 0, "{stderr}");
    assert!(
        stderr.contains("`provider` is not an object"),
        "the CLI replaced a field in the user's config and said nothing: {stderr:?}"
    );
}

#[test]
fn quiet_silences_them() {
    let home = home_with_unnormalizable_opencode();
    let (code, stderr) = run_takeover(home.path(), true);

    assert_eq!(code, 0, "{stderr}");
    assert!(
        !stderr.contains("`provider` is not an object"),
        "--quiet is how a script says it wants nothing but the answer: {stderr:?}"
    );
}
