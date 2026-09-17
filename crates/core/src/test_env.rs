//! Test-only helpers for the environment.
//!
//! Here rather than in one module's `mod tests` because more than one module
//! needs it: anything that reads a variable has to be tested against a value it
//! controls, and the process environment is shared by every test in the binary.

/// One variable set for the duration of a test, put back afterwards.
///
/// The guard must be *bound* (`let _g = EnvGuard::set(…)`) — a bare
/// `EnvGuard::set(…)` as a statement restores the value on the spot.
pub(crate) struct EnvGuard {
    name: &'static str,
    previous: Option<std::ffi::OsString>,
}

impl EnvGuard {
    pub(crate) fn set(name: &'static str, value: Option<&str>) -> Self {
        let previous = std::env::var_os(name);
        match value {
            Some(v) => std::env::set_var(name, v),
            None => std::env::remove_var(name),
        }
        EnvGuard { name, previous }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match self.previous.take() {
            Some(v) => std::env::set_var(self.name, v),
            None => std::env::remove_var(self.name),
        }
    }
}
