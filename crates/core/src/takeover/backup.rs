//! What a takeover captures: the shape of a backup row (`BackupFile`), the
//! rewrite pairs one computes (`Files`), and the file copy that sits beside the
//! row as an escape hatch.

use kiwano_adapters::config::atomic_write_private;
use std::path::Path;

/// The set of backed-up files: `(absolute path, original content)`.
/// The rewritten (path, new content) pairs a takeover writes to disk.
pub(crate) type Files = Vec<(String, String)>;

/// One config file captured by a takeover.
///
/// `existed` separates "was empty" from "was not there". Without it, disabling
/// a takeover writes an empty file back for a config the agent never had —
/// leaving a 0-byte `opencode.json` behind, which the agent may read as a
/// broken config rather than as no config at all.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct BackupFile {
    pub path: String,
    pub content: String,
    pub existed: bool,
}

// (The local mode-less `atomic_write` this file used to carry is gone: every
// takeover file it wrote holds a credential — an auth token, an API key, a
// placeholder — so all of them go through
// `kiwano_adapters::config::atomic_write_private`, which forces 0600 on Unix.)

/// File-copy fallback: `~/.kiwano/backups/<agent>/<filename>` (an escape hatch beyond SQLite).
///
/// Written with `atomic_write_private` like every other file here: a backup copy
/// of `auth.json` or `settings.json` holds the same credential as the original,
/// so it must not land with the process umask's permissions. (Failure is
/// ignored — the copy is a convenience beside the SQLite row, never the
/// authority, so a copy that cannot be written does not fail the takeover.)
pub(crate) fn copy_files(agent: &str, home: &Path, files: &[BackupFile]) {
    let dir = home.join(".kiwano").join("backups").join(agent);
    for file in files {
        // Nothing to copy for a file the agent never had; an empty copy would
        // read as "their config was empty".
        if !file.existed {
            continue;
        }
        let name = std::path::Path::new(&file.path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned());
        if let Some(name) = name {
            let _ = atomic_write_private(&dir.join(name), file.content.as_bytes());
        }
    }
}
