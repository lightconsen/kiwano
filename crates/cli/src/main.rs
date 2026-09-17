//! Thin entry point: hand argv to the library and exit with its code.
//!
//! Everything is in `lib.rs` so `tests/cli.rs` can drive the whole command tree
//! in-process — `run_with` writes to the writers it is given and never calls
//! `exit` or touches the process environment.

use std::io::Write;

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut stdout = std::io::stdout();
    let mut stderr = std::io::stderr();

    // Before `run_with`, not inside it: warnings are emitted while paths are
    // being resolved, which happens before any command runs, and the library is
    // deliberately free of process-global state so its tests can drive it. The
    // quiet flag is read off the arguments here for the same reason — the logger
    // has to exist before the library parses them. A bare scan, which is what
    // the flag is: `--quiet`, no short form, no value.
    kiwano_cli::install_warnings(!argv.iter().any(|arg| arg == "--quiet"));

    let code = kiwano_cli::run_with(&argv, &mut stdout, &mut stderr);
    let _ = stdout.flush();
    let _ = stderr.flush();
    std::process::exit(code);
}
