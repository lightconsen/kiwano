// The session fixtures: one row per source shape, so every branch of the
// table's rendering rules is visible in `npm run dev`. Mirrors
// `crates/api/src/sessions.rs` (`SessionVm`).
//
// The three rows are the three answers the merge can give, and each is here on
// purpose:
//
//   - `both` — the id is in the gateway's traffic *and* in the agent's files,
//     which is the ordinary state of a session that ran after a takeover. The
//     numbers a reader sees come from different sides (traffic for requests and
//     money, files for the project, the turns and the tools), and the `SOURCE`
//     badge is what says so.
//   - `imported` — only a file has it, so there are no requests and no money at
//     all. A zero there would read as "measured zero"; the table prints `—`.
//   - `gateway` — only the gateway saw it, so there is no project and no turns.
//
// The `both` row reuses the session id the request-log fixtures already carry
// (`dev/logs.ts`), so the drill-down from this table lands on real rows.

import type { Session } from "../types";

export const sessions: Session[] = [
  {
    session_id: "user_acct__session_9f3a",
    source: "both",
    agent: "claude",
    project: "acme-api",
    machine: "buildbox",
    title: "port the parser to Rust",
    started_at: "2026-09-09T20:41:00Z",
    ended_at: "2026-09-09T21:30:00Z",
    requests: 3,
    turns: 7,
    input_tokens: 4_210,
    output_tokens: 1_180,
    cache_read_tokens: 12_400,
    cache_creation_tokens: 640,
    // Two currencies on one session is what the column has to keep apart: the
    // traffic spanned two providers, and those bill in different monies.
    cost: [
      { currency: "USD", amount: 2.5 },
      { currency: "CNY", amount: 7.25 },
    ],
    unpriced_rows: 1,
    tool_calls: [
      { name: "Bash", count: 9 },
      { name: "Read", count: 4 },
    ],
    skills: [{ name: "Skill", count: 1 }],
  },
  {
    session_id: "imported-1c2f0a__session",
    source: "imported",
    agent: "claude",
    project: "blog-astro",
    machine: "laptop-two",
    title: "why is the sitemap missing the new posts",
    started_at: "2026-09-06T09:15:00Z",
    ended_at: "2026-09-06T10:02:00Z",
    requests: 0,
    turns: 12,
    input_tokens: 18_900,
    output_tokens: 3_400,
    cache_read_tokens: 0,
    cache_creation_tokens: 0,
    // The files carry tokens, not money: an amount invented from them here
    // would be a guess dressed as a figure.
    cost: [],
    unpriced_rows: 0,
    tool_calls: [{ name: "Edit", count: 6 }],
    skills: [],
  },
  {
    session_id: "gw-only-77ab31",
    source: "gateway",
    agent: "codex",
    started_at: "2026-09-09T22:05:00Z",
    ended_at: "2026-09-09T22:19:00Z",
    requests: 5,
    turns: 0,
    input_tokens: 2_050,
    output_tokens: 900,
    cache_read_tokens: 3_100,
    cache_creation_tokens: 0,
    cost: [{ currency: "USD", amount: 0.42 }],
    unpriced_rows: 0,
    tool_calls: [],
    skills: [],
  },
];
