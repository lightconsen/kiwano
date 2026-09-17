//! The warnings a command emits while it is changing something.
//!
//! Driven through the real binary rather than `run_with`, because that is the
//! only way to observe the stream in question: the library writes to the writers
//! it is handed, while warnings go to the process's stderr through the logger
//! `main` installs. The rest of the command tree is driven in-process (see
//! `cli.rs`); this one file pays for a process to prove the wiring.

use std::path::Path;
use std::process::Command;

/// An isolated home for the spawned binary, on every platform.
///
/// `HOME` is not enough: `get_home_dir` asks the OS (which is the point — see its
/// own notes about `HOME` being injected on Windows), so the environment
/// variable that reaches it is `CC_SWITCH_TEST_HOME`, the hook that exists for
/// exactly this.
const HOME_HOOK: &str = "CC_SWITCH_TEST_HOME";

/// OpenClaw's config root is `~/.openclaw`, with no environment variable that
/// moves it — which is why this test uses that agent: a runner with
/// `XDG_CONFIG_HOME` set (GitHub's Linux runners have it) would otherwise move
/// the file out from under the fixture and the test would fail for a reason that
/// has nothing to do with warnings.
fn home_with_unnormalized_openclaw() -> tempfile::TempDir {
    let home = tempfile::tempdir().unwrap();
    let dir = home.path().join(".openclaw");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("openclaw.json"), r#"{"models":"nope"}"#).unwrap();
    home
}

fn run_takeover(home: &Path, quiet: bool) -> (i32, String) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_kiwano"));
    cmd.env(HOME_HOOK, home)
        .env("KIWANO_DB_PATH", home.join("kiwano.db"))
        .args(["agents", "takeover", "openclaw"]);
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
    let home = home_with_unnormalized_openclaw();
    let (code, stderr) = run_takeover(home.path(), false);

    assert_eq!(code, 0, "{stderr}");
    assert!(
        stderr.contains("`models` is not an object"),
        "the CLI replaced a field in the user's config and said nothing: {stderr:?}"
    );
}

#[test]
fn quiet_silences_them() {
    let home = home_with_unnormalized_openclaw();
    let (code, stderr) = run_takeover(home.path(), true);

    assert_eq!(code, 0, "{stderr}");
    assert!(
        !stderr.contains("`models` is not an object"),
        "--quiet is how a script says it wants nothing but the answer: {stderr:?}"
    );
}
