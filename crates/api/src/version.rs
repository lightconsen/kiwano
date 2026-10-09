//! Which generation of the API a request speaks, and what a daemon accepts.
//!
//! `migrate.local.md` §11.2 asks for three things and this is all of them: a
//! header carrying the generation, a range the daemon publishes, and a refusal
//! that says which side to upgrade. They exist for the **mixed deployment** — an
//! app and a daemon upgraded independently, which is the normal case for a
//! daemon on another machine and a real one even on a desktop (the app ships a
//! daemon, and the old one keeps running until it is replaced).
//!
//! # Why a generation and not a capability list
//!
//! The plan's own words: a per-command capability table is something to
//! maintain, and there is exactly one consumer — our own two front ends. A
//! single integer says "this build and that build can talk" with no table to
//! keep in step. If a third-party client ever exists, that is the moment to
//! reconsider, and not before.

/// The header a client sends and a daemon reads.
pub const HEADER: &str = "x-kiwano-api";

/// The generation **this build** speaks.
///
/// Bumped when a change makes an older peer's requests mean something different
/// — a removed endpoint, a field whose meaning moved, a status code that now
/// means another thing. Adding an endpoint or a field does not bump it: an old
/// client simply never asks for the new one.
pub const CURRENT: u32 = 1;

/// The oldest generation a daemon of this build still answers.
///
/// **0 is accepted on purpose.** A request with no header at all is generation
/// 0: a client built during the migration, which speaks HTTP but predates the
/// header. Those exist in the wild — every build between the first endpoint and
/// this file — and refusing them would break a deployment that is working.
pub const MIN_SUPPORTED: u32 = 0;

/// The generation a request speaks, given the header it carried.
///
/// A missing, unparseable, or negative value is [`MIN_SUPPORTED`]: an old client
/// is the only thing that can produce one, and treating a mangled header as
/// "brand new" would refuse a client that is merely old.
pub fn requested(header: Option<&str>) -> u32 {
    header
        .and_then(|v| v.trim().parse::<u32>().ok())
        .unwrap_or(MIN_SUPPORTED)
}

/// Which side of a version mismatch is behind.
///
/// The whole point of the refusal: "unsupported API version" tells an operator
/// nothing they can act on, while "upgrade the daemon" tells them which machine
/// to touch. Derived rather than passed so the two sides cannot disagree about
/// the direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Behind {
    /// The client is older than the daemon will answer — upgrade the app.
    Client,
    /// The client is newer than this daemon speaks — upgrade the daemon.
    Daemon,
}

/// Whether a daemon that answers `min..=max` can answer a request speaking
/// `wanted`. `None` means yes; `Some(side)` names the side that is behind.
///
/// The range is a parameter rather than read from the constants so both
/// directions can be tested **today**: with a floor of 0 there is no generation
/// below it, so the "the client is behind" branch cannot be reached through
/// [`check`] — and an untestable branch in the code that decides whether two
/// machines can talk is exactly the one that will be wrong when the floor moves.
pub fn check_within(wanted: u32, min: u32, max: u32) -> Option<Behind> {
    if wanted > max {
        Some(Behind::Daemon)
    } else if wanted < min {
        Some(Behind::Client)
    } else {
        None
    }
}

/// [`check_within`] against what this build speaks and accepts.
pub fn check(wanted: u32) -> Option<Behind> {
    check_within(wanted, MIN_SUPPORTED, CURRENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The direction is the part worth testing: it is what the operator acts
    /// on, and getting it backwards sends them to the wrong machine.
    ///
    /// Both directions, with an explicit range — see `check_within`'s note on
    /// why the floor cannot be 0 in a test of this.
    #[test]
    fn the_side_that_is_behind_is_the_one_that_must_move() {
        // A client asking for something newer than the daemon speaks: the
        // daemon is behind.
        assert_eq!(check_within(3, 1, 2), Some(Behind::Daemon));
        // A client below the floor the daemon answers: the app is behind.
        assert_eq!(check_within(0, 1, 2), Some(Behind::Client));
        // Inside the range, both edges included.
        assert_eq!(check_within(1, 1, 2), None);
        assert_eq!(check_within(2, 1, 2), None);
        // And through the real constants: everything this build speaks, it
        // answers, and the floor it accepts is the one a headerless client gets.
        assert_eq!(check(CURRENT), None);
        assert_eq!(check(MIN_SUPPORTED), None);
        assert_eq!(check(CURRENT + 1), Some(Behind::Daemon));
    }

    /// A header nobody can parse is an old client, not a new one.
    ///
    /// The alternative — treating garbage as the newest generation — would
    /// refuse exactly the clients least able to explain themselves.
    #[test]
    fn an_unreadable_header_is_treated_as_the_oldest_client() {
        assert_eq!(requested(None), MIN_SUPPORTED);
        assert_eq!(requested(Some("")), MIN_SUPPORTED);
        assert_eq!(requested(Some("  ")), MIN_SUPPORTED);
        assert_eq!(requested(Some("two")), MIN_SUPPORTED);
        assert_eq!(requested(Some("-1")), MIN_SUPPORTED);
        // And a real one is read, whitespace and all.
        assert_eq!(requested(Some(" 1 ")), 1);
    }
}
