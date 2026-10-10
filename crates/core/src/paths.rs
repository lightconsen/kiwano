//! Where Kiwano's own files live, and where the user's do.
//!
//! The front ends (the app and the CLI) both need the user's home, because it
//! roots every agent config a takeover reads or writes. They used to read `HOME`
//! themselves, with `.` as the fallback — and `kiwano_adapters::config` warns
//! against exactly that: on Windows `HOME` can be injected by Git/MSYS, and a
//! home that is not the profile directory silently relocates every agent's
//! config. [`home_dir`] is the one that asks the OS instead.

use std::path::PathBuf;

/// The database path and whatever had to be ignored to resolve it, so a front
/// end can name the type without depending on the adapter crate directly.
pub use kiwano_adapters::config::DbPath;

/// The user's home directory.
pub fn home_dir() -> PathBuf {
    kiwano_adapters::config::get_home_dir()
}

/// A **best-effort label** for this machine — what an imported row records as its
/// origin (`imported_from`, migration v32).
///
/// This is a label, **not an identity**. Two machines can share a name, a name
/// can change, and there is nothing here to make it unique; the point is only
/// that a human reading a merged ledger from two laptops can tell one machine's
/// history from the other's. It is the honest answer to the same question the
/// client keys ask — "who is spending" — for the one case where no key exists:
/// an imported row predates the gateway and carries no credential, so a name is
/// all there is.
///
/// Best effort, and no new dependency: `HOSTNAME` then `COMPUTERNAME` (the two
/// spellings between them cover the platforms), then the `hostname` command. A
/// subprocess is fine here — this crate runs as the user, in a client, while the
/// *daemon* is the side that is kept from spawning anything. `None` when nothing
/// answers, which is honest rather than a guess: a blank string is not a name,
/// and the display has a word for "unnamed machine" (`unnamed`) rather than a
/// blank cell.
pub fn machine_name() -> Option<String> {
    machine_name_from(
        ["HOSTNAME", "COMPUTERNAME"]
            .iter()
            .filter_map(|v| std::env::var(v).ok()),
        hostname_command,
    )
}

/// The decision procedure [`machine_name`] is, split out so its contract is
/// observable: given the candidates the environment offers and the command that
/// may answer, it is `Some(name)` for the first non-blank candidate (trimmed) and
/// `None` when nothing answers at all. A blank candidate is not an answer.
///
/// The command is only asked when the environment had nothing, so the common case
/// spawns nothing.
fn machine_name_from(
    env: impl Iterator<Item = String>,
    command: impl FnOnce() -> Option<String>,
) -> Option<String> {
    for raw in env {
        if let Some(name) = machine_label(&raw) {
            return Some(name);
        }
    }
    command().and_then(|raw| machine_label(&raw))
}

/// Trim a candidate and refuse it if blank — the whole of "a name or nothing".
fn machine_label(raw: &str) -> Option<String> {
    let label = raw.trim();
    (!label.is_empty()).then(|| label.to_string())
}

/// The `hostname` command's stdout, trimmed by [`machine_label`]; `None` when it
/// cannot be run or exits non-zero. `hostname` exists on every platform this
/// ships for (it is a shell builtin on some, but a binary too).
fn hostname_command() -> Option<String> {
    let out = std::process::Command::new("hostname").output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The **client's own** database and whatever had to be ignored to resolve it
/// (`migrate.local.md` §9.5 step 2).
///
/// Beside [`crate::sidecar::db_path`] because the two are a pair a front end
/// resolves together: the shared file is the daemon's and this one is the
/// client's, and which is which is the whole of the boundary.
pub fn local_db_path(shared: &std::path::Path) -> DbPath {
    kiwano_adapters::config::kiwano_local_db_path(shared)
}

#[cfg(test)]
mod tests {
    use super::machine_name_from;

    /// The contract that matters: when nothing answers, the answer is `None` and
    /// **not an empty string**. A blank or whitespace-only environment variable is
    /// not a name (a variable set to `""` is common on Windows and in CI), and a
    /// command that prints nothing is not one either — so the display can print
    /// its own word for an unnamed machine rather than a blank cell.
    #[test]
    fn nothing_answering_is_none_not_an_empty_name() {
        assert_eq!(
            machine_name_from(["  ".to_string(), String::new()].into_iter(), || None),
            None,
            "a blank variable is no more a name than an absent one"
        );
        assert_eq!(
            machine_name_from(std::iter::empty(), || Some("\n".to_string())),
            None,
            "a command that prints only a newline has told us nothing"
        );
    }

    /// And a name that does answer survives, from either source, trimmed — the
    /// environment first (it costs no process), the command only when it is empty.
    #[test]
    fn a_source_that_answers_is_trimmed_and_wins() {
        assert_eq!(
            machine_name_from(["  buildbox  ".to_string()].into_iter(), || {
                panic!("the environment answered; the command must not be run")
            }),
            Some("buildbox".to_string())
        );
        assert_eq!(
            machine_name_from(std::iter::empty(), || Some(" box\n".to_string())),
            Some("box".to_string())
        );
    }
}
