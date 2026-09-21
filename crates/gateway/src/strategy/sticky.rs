//! Session stickiness: the table that remembers where a session was sent, and
//! the plumbing every strategy that pins through it shares.
//!
//! A conversation's upstream prompt cache is worth more than the round-robin
//! fairness or the quota spread that moving it mid-flight would buy, so every
//! strategy whose answer can change while a conversation is running asks
//! `pinned` (or, for the two that only drain, `drain`) before it asks anything
//! else. The table is bounded — see `STICKY_CAPACITY` — because one entry per
//! session held for the life of the process is a leak.

use std::collections::HashMap;

use crate::router::AgentRoute;

use super::StrategyEngine;

/// Sticky-table key for cross-session rotation: `agent \0 session` (\0 avoids concatenation ambiguity).
fn sticky_key(agent: &str, session: Option<&str>) -> String {
    format!("{agent}\u{0}{}", session.unwrap_or(""))
}

/// One session's assignment: which candidate, and when it was last used.
pub(crate) struct Sticky {
    /// The candidate's **id**, not its position. The candidate list is pruned
    /// per request (a provider over a billing limit is dropped before the
    /// strategy is asked), so a stored index means a different provider the
    /// moment anything ahead of it is pruned — and a session pinned to the wrong
    /// provider is exactly the cache loss this table exists to prevent.
    id: String,
    /// The tick of the last request that used this entry. Ordering by it is what
    /// makes the table least-recently-used rather than oldest-first.
    used: u64,
}

/// How many sessions keep their assignment. An entry is a session key and an
/// index — tens of bytes — so this is a few hundred kilobytes at worst, which
/// is cheaper than the bookkeeping it would take to size it exactly.
pub(crate) const STICKY_CAPACITY: usize = 4096;

/// roundrobin's session table, with a ceiling.
///
/// The ceiling is the point: a session can run for hours and there can be many
/// of them, so one entry per session, kept for the life of the process, is a map
/// that only grows. A session that is still talking is touched by every request
/// it makes, so least-recently-used eviction cannot take the slot out from under
/// a live conversation — only one that has gone quiet, which is a conversation
/// that has ended (or one that will pay a single cold prefix if it wakes up).
pub(crate) struct StickyTable {
    pub(crate) map: HashMap<String, Sticky>,
    tick: u64,
}

impl StickyTable {
    pub(crate) fn new() -> Self {
        StickyTable {
            map: HashMap::new(),
            tick: 0,
        }
    }

    /// The candidate this key is assigned to, if any — and marking that as the
    /// moment it was used. A read that hits is a use: this is what keeps a live
    /// session out of the eviction scan.
    ///
    /// The id comes back owned rather than borrowed: the caller has to await
    /// before it can use it, and a guard held across an await makes the future
    /// non-Send. An id is a few dozen bytes.
    fn get(&mut self, key: &str) -> Option<String> {
        self.tick += 1;
        let tick = self.tick;
        let entry = self.map.get_mut(key)?;
        entry.used = tick;
        Some(entry.id.clone())
    }

    fn insert(&mut self, key: String, id: String) {
        self.tick += 1;
        self.map.insert(
            key,
            Sticky {
                id,
                used: self.tick,
            },
        );
        self.evict_if_over();
    }

    /// Drop the quietest quarter in one pass. A quarter rather than the single
    /// oldest entry because the scan is the cost and a batch amortizes it, and a
    /// quarter rather than everything because the survivors are the ones being
    /// used — among them, almost certainly, the session that just arrived.
    fn evict_if_over(&mut self) {
        if self.map.len() <= STICKY_CAPACITY {
            return;
        }
        let mut ages: Vec<u64> = self.map.values().map(|s| s.used).collect();
        ages.sort_unstable();
        let cutoff = ages[ages.len() / 4];
        self.map.retain(|_, s| s.used > cutoff);
    }
}

impl StrategyEngine {
    /// The candidate this session is already on, if it is still usable.
    ///
    /// Session-granularity stickiness is what keeps a conversation's upstream
    /// prompt cache intact, and it belongs to every strategy whose answer can
    /// change while a conversation is running: roundrobin's ring, the quota
    /// threshold, a closing time window, a provider recovering from a failure.
    ///
    /// A *new* session asks the strategy and goes wherever it says. A session
    /// already running stays put, because the alternative is rebuilding a prefix
    /// the upstream has already cached — and what the switch buys (spreading a
    /// quota, honouring a window, returning to the preferred provider) can wait
    /// for the conversation to end. The cost of waiting is bounded by the
    /// conversation; the cost of switching is paid in this turn's tokens.
    ///
    /// What drain does *not* do is spend past a ceiling. The candidate list this
    /// looks in has already had the providers over a billing limit pruned out of
    /// it, and a breaker that has opened fails the availability check below —
    /// so a pin holds only while the provider is genuinely usable, and a hard
    /// limit still wins over a conversation in flight.
    pub(crate) async fn pinned(
        &self,
        route: &AgentRoute,
        session: Option<&str>,
    ) -> Option<crate::router::UpstreamProvider> {
        let key = sticky_key(&route.agent, session);
        // The lock only guards the table read (a guard must not be held across
        // await, or the future is not Send).
        let id = self.sticky.lock().expect("sticky map poisoned").get(&key)?;
        let idx = route.candidates.iter().position(|c| c.id == id)?;
        if self.candidate_available(route, idx).await {
            return Some(route.candidates[idx].clone());
        }
        None
    }

    /// [`Self::pinned`] for the strategies that *drain*: pin a conversation, and
    /// only a conversation.
    ///
    /// A request that names no session cannot be drained, because nothing can
    /// tell a continuing conversation from a new one — and the table's key for
    /// "no session" is shared by every such request of an agent, so pinning it
    /// would hold the agent's entire un-named traffic on whichever provider
    /// answered first and the strategy would never fire again. Those requests
    /// are chosen afresh every time, which is what they got before.
    pub(crate) async fn drain(
        &self,
        route: &AgentRoute,
        session: Option<&str>,
    ) -> Option<crate::router::UpstreamProvider> {
        let session = session?;
        self.pinned(route, Some(session)).await
    }

    /// Record where a session was sent — the other half of [`Self::drain`], and
    /// silent for a request that named none.
    pub(crate) fn assign_drained(
        &self,
        route: &AgentRoute,
        session: Option<&str>,
        provider: &crate::router::UpstreamProvider,
    ) {
        if let Some(session) = session {
            self.assign(route, Some(session), provider);
        }
    }

    /// Record where a session was sent, so its next request can stay there.
    pub(crate) fn assign(
        &self,
        route: &AgentRoute,
        session: Option<&str>,
        provider: &crate::router::UpstreamProvider,
    ) {
        self.sticky
            .lock()
            .expect("sticky map poisoned")
            .insert(sticky_key(&route.agent, session), provider.id.clone());
    }
}
