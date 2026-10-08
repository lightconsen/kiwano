//! The failure a resource endpoint answers with.
//!
//! Until the migration there was no error type on the wire, because there was
//! no wire: `vm::` functions returned `Result<_, String>` and the app showed the
//! string. A separated daemon breaks that in a specific way — the daemon has to
//! choose an HTTP status, and it cannot choose one from prose. The first version
//! of the keys endpoints did the only thing prose allows: it *re-queried the
//! store* to find out whether the 400 or the 404 was the right answer, which is
//! a second source of truth for a decision the function being served had already
//! made (`migrate.local.md` §10.8).
//!
//! So the failure carries a **kind**: a small closed vocabulary a program
//! branches on, plus the message a person reads. The daemon maps the kind to a
//! status; a client maps it to a retry decision; the UI shows the message, which
//! is the *same* string it showed before — the contract fixtures pin those, so
//! they travel verbatim rather than being reworded by the move.
//!
//! `migrate.local.md` §6.1 asked for exactly this ("turn the failure vocabulary
//! into a stable `kind` vocabulary rather than prose"). It is three variants
//! because three are what the moved commands actually need; a kind nothing
//! returns is a shape guessed ahead of its use.

use serde::{Deserialize, Serialize};

/// What went wrong, in the form a caller can branch on.
///
/// Deliberately *not* an HTTP status: this crate has no HTTP in it, and a
/// status code would make the contract transitive through a protocol. The
/// mapping lives with the server, which is the only side that has one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiErrorKind {
    /// The request named something that is not there — an unknown provider, a
    /// key id with no row. The request was well formed; the world was not what
    /// it assumed.
    NotFound,
    /// The request was understood and refused on its own terms: an empty key,
    /// a strategy type that does not exist. Repeating it will fail the same way.
    Invalid,
    /// The storage layer said no. Nothing about the request explains it, so it
    /// is the one kind worth retrying.
    Failed,
}

/// A failure, as the wire carries it.
///
/// Serialized as `{"kind": "not_found", "message": "…"}` — the shape the
/// daemon's error envelope already had room for, now with a machine-readable
/// field beside the sentence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiError {
    pub kind: ApiErrorKind,
    /// What to show a person. For a moved command this is byte-for-byte the
    /// string the `vm::` function returned, because that is what the UI has
    /// always displayed and what the contract fixtures freeze.
    pub message: String,
}

impl ApiError {
    pub fn not_found(message: impl Into<String>) -> Self {
        Self {
            kind: ApiErrorKind::NotFound,
            message: message.into(),
        }
    }

    /// Refused on the request's own terms — a bad field, an unknown strategy
    /// name, a missing required argument.
    pub fn invalid(message: impl Into<String>) -> Self {
        Self {
            kind: ApiErrorKind::Invalid,
            message: message.into(),
        }
    }

    /// Anything the request cannot explain.
    ///
    /// Takes the error rather than a string so callers do not each invent a
    /// format for it: this is what `e2s` did in `kiwano-core`, in one place and
    /// for both sides.
    pub fn failed(cause: impl std::fmt::Display) -> Self {
        Self {
            kind: ApiErrorKind::Failed,
            message: cause.to_string(),
        }
    }

    pub fn kind(&self) -> ApiErrorKind {
        self.kind
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ApiError {}

#[cfg(test)]
mod tests {
    use super::*;

    /// The kind is a wire value, so its spelling is part of the contract: a
    /// client matches on these strings. Pinned here rather than left to the
    /// first integration test to notice.
    #[test]
    fn the_kind_spellings_are_frozen() {
        let as_wire = |kind: ApiErrorKind| {
            serde_json::to_value(ApiError {
                kind,
                message: String::new(),
            })
            .unwrap()["kind"]
                .as_str()
                .unwrap()
                .to_string()
        };
        assert_eq!(as_wire(ApiErrorKind::NotFound), "not_found");
        assert_eq!(as_wire(ApiErrorKind::Invalid), "invalid");
        assert_eq!(as_wire(ApiErrorKind::Failed), "failed");
    }

    /// The message survives the round trip unchanged — a client shows what the
    /// far end said, not a re-rendering of it.
    #[test]
    fn an_error_round_trips_with_its_message() {
        let err = ApiError::not_found("provider `p-1` not found");
        let json = serde_json::to_string(&err).unwrap();
        assert_eq!(serde_json::from_str::<ApiError>(&json).unwrap(), err);
        // `Display` is the message alone: the app's IPC is a `Result<_, String>`
        // and this is what fills the `String`.
        assert_eq!(err.to_string(), "provider `p-1` not found");
    }
}
