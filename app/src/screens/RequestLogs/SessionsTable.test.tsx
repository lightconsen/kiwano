// The session row's turn count, and where it is allowed to come from.
//
// A turn in a file is not a request, so the two ledgers each have their own
// count — but a session only the gateway saw used to have no answer at all,
// which is what the column of `—` in the bug report was. Since v38 the daemon
// counts the assistant turns in each chat-shaped request body, and the table
// shows that count for a traffic-only row too.
import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { Session } from "@/api/types";
import { rememberCustomAgents } from "@/lib/agents";

import { SessionsTable } from "./SessionsTable";

/** One session row, `both` by default; `over` picks the shape under test. */
function session(id: string, over: Partial<Session> = {}): Session {
  return {
    session_id: id,
    source: "both",
    agent: "claude",
    project: "acme-api",
    machine: "buildbox",
    title: `session ${id}`,
    started_at: "2026-09-14T09:00:00Z",
    ended_at: "2026-09-14T10:00:00Z",
    requests: 2,
    turns: 5,
    input_tokens: 1_000,
    output_tokens: 200,
    cache_read_tokens: 0,
    cache_creation_tokens: 0,
    cost: [{ currency: "USD", amount: 2.5 }],
    unpriced_rows: 0,
    tool_calls: [],
    tool_uses: { head: [], tail: [], skipped: 0 },
    skills: [],
    ...over,
  };
}

function renderRows(rows: Session[]) {
  render(
    <SessionsTable
      rows={rows}
      loaded
      managedAgents={["claude"]}
      onOpenSession={() => {}}
    />,
  );
}

describe("the TURNS column", () => {
  it("answers for a traffic-only session from the requests themselves", () => {
    renderRows([
      session("s-gw", {
        source: "gateway",
        project: null,
        title: null,
        requests: 12,
        turns: 42,
      }),
    ]);
    // The count the daemon measured, not a dash and not the request count.
    expect(screen.getByText("42")).toBeInTheDocument();
    expect(screen.getByText("12")).toBeInTheDocument();
  });

  it("still shows — when no request carried a count", () => {
    // A Responses-shaped body has no `messages` array, and rows from before
    // the column existed have none either: both arrive as 0, which is
    // "not measured" rather than a conversation with zero turns.
    renderRows([
      session("s-gw-none", {
        source: "gateway",
        project: null,
        title: null,
        turns: 0,
      }),
    ]);
    const cells = screen.getAllByRole("cell");
    expect(cells.some((c) => c.textContent === "—")).toBe(true);
    expect(cells.some((c) => c.textContent === "0")).toBe(false);
  });

  it("names a custom agent the way the app does, on one line", () => {
    // `hello-92fc46` is a real id here: the label is what the reader chose,
    // and the id is only how the route is addressed — so the cell shows the
    // label and the tooltip keeps the id. One line, every row.
    rememberCustomAgents([{ id: "hello-92fc46", label: "Hello" }]);
    renderRows([session("s-custom", { source: "gateway", agent: "hello-92fc46" })]);
    const cell = screen.getByTitle("hello-92fc46");
    expect(cell.textContent).toBe("Hello");
    expect(cell.firstElementChild?.className).toContain("truncate");
  });

  it("keeps the files' count when both sides have one", () => {
    renderRows([session("s-both", { turns: 12 })]);
    expect(screen.getByText("12")).toBeInTheDocument();
  });
});
