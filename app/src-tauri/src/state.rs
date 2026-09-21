//! The state `run` manages, and the reload ping a mutation sends through it.
//!
//! `AppState` is here rather than in the facade because it belongs to the whole
//! tree: `app.manage` hands it to the spawn tasks, and 44 of the 52 commands
//! read the store, the auxiliary connection or the admin endpoint out of it.
//! Its fields and its one method are `pub(crate)` for that reason, and so is
//! [`after_mutation`] — twelve writes across six modules end with it. The type
//! itself is `pub` because the commands that take it are: a `pub` function
//! cannot name a `pub(crate)` type, and every one of them is re-exported for
//! `generate_handler!`.

use std::sync::{Mutex, OnceLock};

use kiwanod::store::Store;
use tauri::State;

use crate::update;
use kiwano_core::{detect, sidecar, vm};
use sidecar::AdminEndpoint;
use vm::Aux;

pub struct AppState {
    pub(crate) store: Store,
    pub(crate) aux: Aux,
    pub(crate) child: Mutex<Option<std::process::Child>>,
    /// Where the admin plane is, not a port: a socket beside the database, or a
    /// per-user pipe on Windows. Resolved once at startup and shared by every
    /// caller, so nothing re-reads the environment mid-session.
    pub(crate) admin: AdminEndpoint,
    pub(crate) data_port: u16,
    /// Update found by the silent startup check, so the UI can show it without
    /// the user running a check by hand (see `get_pending_update`).
    pub(crate) pending_update: Mutex<Option<update::UpdateInfoVm>>,
    /// The variables that move where an agent keeps its files, as the user's own
    /// login shell has them — asked for on first use rather than at startup, so
    /// a slow interactive rc file is paid for by the first screen that needs it
    /// rather than by the window opening.
    ///
    /// It has to come from the shell: the app's environment is launchd's, and a
    /// `CLAUDE_CONFIG_DIR` exported in `.zshrc` is not in it. See
    /// `detect::login_shell_vars`.
    pub(crate) shell_vars: OnceLock<detect::ShellVars>,
}

impl AppState {
    /// The shell's answer, resolved once per run.
    pub(crate) fn shell_vars(&self) -> &detect::ShellVars {
        self.shell_vars
            .get_or_init(|| detect::login_shell_vars(kiwano_core::takeover::CONFIG_DIR_VARS))
    }
}

pub(crate) fn after_mutation(state: &State<AppState>) {
    sidecar::notify_reload(&state.admin);
}
