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

/** An instant `daysAgo` days back, written the way the daemon writes timestamps
 *  (`+00:00`, no milliseconds).
 *
 * Both halves of that matter. The fixtures used to carry fixed September dates,
 * which meant the embedded demo's Sessions tab was **empty** for every visitor:
 * the window selector above the card defaults to seven days and the fixtures fell
 * outside it. And the spelling is not cosmetic — the bound this is compared
 * against is built by `localMidnightUtc`, so a `.000Z` stamp would sort *after*
 * its own instant and drop the row sitting exactly on the boundary.
 */
function daysAgo(days: number, hour = 10, minute = 0): string {
  const at = new Date();
  at.setDate(at.getDate() - days);
  at.setHours(hour, minute, 0, 0);
  return at.toISOString().replace(/\.\d{3}Z$/, "+00:00");
}

export const sessions: Session[] = [
  {
    session_id: "user_acct__session_9f3a",
    source: "both",
    agent: "claude",
    project: "acme-api",
    machine: "buildbox",
    title: "port the parser to Rust",
    started_at: daysAgo(1, 20, 41),
    ended_at: daysAgo(1, 21, 30),
    requests: 3,
    turns: 7,
    // The files' own count of the same conversation: close to the gateway's and
    // deliberately not equal, which is what two ledgers of one session look like.
    file_tokens: { input: 4_500, output: 1_240, cache_read: 11_900, cache_creation: 700 },
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
    // In the order they happened, with what each one did — enough of them that
    // the table has a second page, which is the shape a real session of this
    // size has (the one this was built against made 1,817 calls).
    tool_uses: {
      head: [
      { name: "Read", detail: "/Users/me/work/acme-api/src/parser.rs", at: daysAgo(1, 20, 41) },
      { name: "Grep", detail: "fn parse_token", at: daysAgo(1, 20, 42) },
      { name: "Bash", detail: "cargo test --lib parser", at: daysAgo(1, 20, 43) },
      { name: "Edit", detail: "/Users/me/work/acme-api/src/parser.rs", at: daysAgo(1, 20, 44) },
      { name: "Bash", detail: "cargo test --lib parser", at: daysAgo(1, 20, 45) },
      { name: "Read", detail: "/Users/me/work/acme-api/src/lexer.rs", at: daysAgo(1, 20, 46) },
      { name: "Edit", detail: "/Users/me/work/acme-api/src/lexer.rs", at: daysAgo(1, 20, 47) },
      { name: "Bash", detail: "cargo clippy -- -D warnings", at: daysAgo(1, 20, 48) },
      { name: "Agent", detail: "Explore the tokenizer's error paths", at: daysAgo(1, 20, 49) },
      { name: "Edit", detail: "/Users/me/work/acme-api/src/parser.rs", at: daysAgo(1, 20, 50) },
      { name: "Bash", detail: "cargo test --lib parser", at: daysAgo(1, 20, 51) },
      { name: "Write", detail: "/Users/me/work/acme-api/tests/parser_cases.rs", at: daysAgo(1, 20, 52) },
      { name: "Bash", detail: "cargo test --test parser_cases", at: daysAgo(1, 20, 53) },
      { name: "Read", detail: "/Users/me/work/acme-api/src/parser.rs", at: daysAgo(1, 21, 29) },
      ],
      tail: [],
      skipped: 0,
    },
    skills: [{ name: "Skill", count: 1 }],
  },
  {
    session_id: "imported-1c2f0a__session",
    source: "imported",
    agent: "claude",
    project: "blog-astro",
    machine: "laptop-two",
    title: "why is the sitemap missing the new posts",
    started_at: daysAgo(4, 9, 15),
    ended_at: daysAgo(4, 10, 2),
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
    tool_uses: {
      head: [
        { name: "Edit", detail: "/Users/me/blog-astro/src/content/config.ts", at: "2026-10-08T09:20:00Z" },
        { name: "Write", detail: "/Users/me/blog-astro/src/pages/sitemap.xml.ts", at: "2026-10-08T09:48:00Z" },
      ],
      tail: [],
      skipped: 0,
    },
    skills: [],
  },
  {
    session_id: "gw-only-77ab31",
    source: "gateway",
    agent: "codex",
    started_at: daysAgo(0, 8, 5),
    ended_at: daysAgo(0, 8, 19),
    requests: 5,
    turns: 0,
    input_tokens: 2_050,
    output_tokens: 900,
    cache_read_tokens: 3_100,
    cache_creation_tokens: 0,
    cost: [{ currency: "USD", amount: 0.42 }],
    unpriced_rows: 0,
    tool_calls: [],
    tool_uses: { head: [], tail: [], skipped: 0 },
    skills: [],
  },
];
