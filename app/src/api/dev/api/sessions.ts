// The session list, against the dev fixtures. Mirrors
// `crates/gateway/src/api/sessions.rs::list_sessions` — one predicate, so the
// window and the agent filter mean here what they mean there.

import type { KiwanoApi, Session, SessionList } from "../../types";
import { delay } from "../delay";
import { sessions } from "../sessions";

/** Newest first, and bounded by the instant the caller computed.
 *
 * `since` is an instant rather than a day count because that is what the real
 * route takes from a client showing a local-day window (see `SessionQuery`);
 * `days` is the CLI's rolling spelling and is not used here. The bound is
 * compared as a string, exactly as SQLite compares the RFC3339 column — and it
 * bounds `ended_at`, the session's last activity, which is the field the store's
 * own filter uses. */
export function matchingSessions(
  agent?: string,
  since?: string,
): Session[] {
  return sessions
    .filter((s) => (!agent || s.agent === agent) && (!since || s.ended_at >= since))
    .sort((a, b) => (a.ended_at < b.ended_at ? 1 : -1));
}

export const sessionsApi: Pick<KiwanoApi, "listSessions" | "getSession"> = {
  async getSession(id: string): Promise<Session | null> {
    await delay();
    // The same fixture list the index reads, so a session the list shows is a
    // session the detail page can open — in dev and in the demo alike.
    return sessions.find((s) => s.session_id === id) ?? null;
  },

  async listSessions(
    agent: string | undefined,
    _project: string | undefined,
    _days: number | undefined,
    since: string | undefined,
    agents: string[] | undefined,
    page: number,
    pageSize: number,
  ): Promise<SessionList> {
    await delay();
    // `_project` is accepted and ignored: the fixtures carry project labels, but
    // nothing in the app sets a project filter yet — the parameter is here
    // because the daemon's route takes it.
    // The mock is the dev fixture, so it is always the "developing" case: the
    // real filtering happens in the daemon, and a mock that reproduced it would
    // be a second copy of a rule with nothing keeping the two in step — except
    // when a test asks it to, which is what `agents` here is for.
    const all = matchingSessions(agent, since).filter(
      (s) => !agents || agents.includes(s.agent),
    );
    const from = Math.max(0, (Math.max(1, page) - 1) * pageSize);
    return { rows: all.slice(from, from + pageSize), total: all.length };
  },
};
