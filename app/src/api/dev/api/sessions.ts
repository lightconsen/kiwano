// The session list, against the dev fixtures. Mirrors
// `crates/gateway/src/api/sessions.rs::list_sessions` — one predicate, so the
// window and the agent filter mean here what they mean there.

import type { KiwanoApi, Session } from "../../types";
import { delay } from "../delay";
import { sessions } from "../sessions";

/** Newest first, and bounded by the instant the caller computed.
 *
 * `since` is an instant rather than a day count because that is what the real
 * route takes from a client showing a local-day window (see `SessionQuery`);
 * `days` is the CLI's rolling spelling and is not used here. The bound is
 * compared as a string, exactly as SQLite compares the RFC3339 column — and it
 * bounds `started_at`, which is the field the store's own filter uses. */
export function matchingSessions(
  agent?: string,
  since?: string,
): Session[] {
  return sessions
    .filter((s) => (!agent || s.agent === agent) && (!since || s.started_at >= since))
    .sort((a, b) => (a.ended_at < b.ended_at ? 1 : -1));
}

export const sessionsApi: Pick<KiwanoApi, "listSessions"> = {
  async listSessions(agent?: string, _project?: string, _days?: number, since?: string) {
    await delay();
    // `_project` is accepted and ignored: the fixtures carry project labels, but
    // nothing in the app sets a project filter yet — the parameter is here
    // because the daemon's route takes it.
    return matchingSessions(agent, since);
  },
};
