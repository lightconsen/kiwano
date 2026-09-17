//! One log file, written by both processes.
//!
//! The app and the gateway are separate processes that fail in different ways,
//! and a packaged app is launched by launchd — where stdout goes to `/dev/null`
//! — so until this existed, the only way to read either one was to start it
//! from a terminal. Everything a user would need to explain a failure
//! (a provider falling out of the route table, a sync that never got through,
//! the sidecar failing to start) was being thrown away.

use std::path::{Path, PathBuf};

use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::prelude::*;

/// Days of history to keep. Enough to cover "it broke a few days ago" without
/// needing anyone to think about it.
const KEEP_DAYS: usize = 7;

/// Warnings to stderr, for a process with no log file of its own: the CLI.
///
/// `warn` by default, so an ordinary run stays quiet, and `RUST_LOG` raises it
/// when someone is debugging — the same variable the gateway honours. What this
/// is for is the adapter crate's warnings, which fire while a command is
/// *changing* something (a config normalized, a field replaced) and which the
/// CLI would otherwise discard: a mutation the user is told nothing about is the
/// one kind of quiet output that is not acceptable.
///
/// Nothing is installed when the run is quiet — a `log` record with no logger is
/// dropped, which is exactly what `--quiet` asks for.
pub fn init_stderr_warnings(quiet: bool) {
    if quiet {
        return;
    }
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn"));
    // `try_init` rather than `init`: this is the CLI's process, where a second
    // call (a test driving the binary, an embedded invocation) should not panic
    // over a subscriber that is already doing the job.
    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(std::io::stderr)
                // A terminal line does not need the date, and the module path is
                // what says where it came from.
                .without_time(),
        )
        .try_init();
}

/// A `logs/` directory beside the database, so a scratch database takes its
/// logs with it and nothing has to be found twice.
pub fn log_dir(db_path: &Path) -> PathBuf {
    db_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("logs")
}

/// Install the global subscriber: `info` and above to `logs/kiwano.log.<date>`
/// (rotated daily, seven kept), plus panic messages, plus a copy on stdout so a
/// terminal run reads exactly as it did before.
///
/// `RUST_LOG` overrides the level, the same variable the gateway always
/// honoured. Every line carries its module path, which is what tells an app
/// line from a gateway one.
///
/// Returns the directory, or `None` when it could not be created — a program
/// that cannot log should still run, and stdout remains.
pub fn init(db_path: &Path) -> Option<PathBuf> {
    let dir = log_dir(db_path);
    if std::fs::create_dir_all(&dir).is_err() {
        return None;
    }
    let appender = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix("kiwano.log")
        .max_log_files(KEEP_DAYS)
        .build(&dir)
        .ok()?;

    // Leaked on purpose: a layer's writer has to outlive the subscriber, which
    // is `'static` once installed, and the leak is one appender for the life of
    // the process. The alternative — the non-blocking writer — hands lines to a
    // background thread, and a panic with `panic = "abort"` exits without it
    // ever flushing: the crash would be the one event missing from the log.
    let writer: &'static RollingFileAppender = Box::leak(Box::new(appender));

    let filter = || {
        tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"))
    };
    tracing_subscriber::registry()
        .with(filter())
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stdout))
        .with(
            tracing_subscriber::fmt::layer()
                // A closure, not the appender: `&RollingFileAppender` only
                // implements MakeWriter for one lifetime, while `Fn() -> impl
                // Write` covers every one.
                .with_writer(move || writer.make_writer())
                .with_ansi(false),
        )
        .init();

    // A panic is the case the log exists for, and by default it goes to stderr
    // with the rest.
    std::panic::set_hook(Box::new(|info| {
        tracing::error!(panic = %info, "panicked");
    }));

    Some(dir)
}
#[cfg(test)]
mod tests {
    use super::*;

    /// The `log` facade is what the adapter crate logs through, and a record
    /// emitted there has to reach the same file as a `tracing` one — otherwise
    /// "the home directory could not be determined" and the rest of what the
    /// adapters say are written nowhere at all.
    #[test]
    fn a_log_record_reaches_the_shared_file() {
        let dir = std::env::temp_dir().join(format!("kiwano-log-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let logs = init(&dir.join("kiwano.db")).expect("the log directory is writable");
        log::warn!("a line from the adapters");

        // The daily appender writes `kiwano.log.<date>`; whichever file it made
        // is the one to read.
        let written = std::fs::read_dir(&logs)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| std::fs::read_to_string(e.path()).unwrap_or_default())
            .collect::<String>();
        assert!(
            written.contains("a line from the adapters"),
            "the adapter's own logging goes nowhere: {written:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
