// One session's own page: what it shows, and the rules it has to keep while
// showing it.
//
// The rules are the interesting part — a session's numbers come from two
// ledgers, and this page is the first place with room to say so. So the tests
// are mostly about what must *not* happen: nothing summed, no zero where a side
// never looked, no money converted.
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { en } from "@/i18n/en";
import type { Session } from "@/api/types";

import SessionDetail from "./SessionDetail";

const { apiMock, onShowRequests } = vi.hoisted(() => ({
  apiMock: { getSession: vi.fn() },
  onShowRequests: vi.fn(),
}));

vi.mock("../api/client", () => ({ api: apiMock }));

/** ONE_SHAPE: a session both ledgers know — the ordinary state after a takeover. */
function session(over: Partial<Session> = {}): Session {
  return {
    session_id: "s-1",
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
    cost: [{ currency: "USD", amount: 2.5 }],
    unpriced_rows: 0,
    tool_calls: [],
    tool_uses: { head: [], tail: [], skipped: 0 },
    skills: [],
    ...over,
  };
}

/** Calls in the shape the reader keeps: a head, an optional tail, a gap. */
function calls(
  head: { name: string; detail: string; at?: string }[],
  tail: typeof head = [],
  skipped = 0,
) {
  const stamp = (i: number) => `2026-09-09T20:${String(41 + i).padStart(2, "0")}:00Z`;
  return {
    head: head.map((c, i) => ({ ...c, at: c.at ?? stamp(i) })),
    tail: tail.map((c, i) => ({ ...c, at: c.at ?? stamp(head.length + skipped + i) })),
    skipped,
  };
}

function renderDetail(id = "s-1") {
  return render(<SessionDetail sessionId={id} onShowRequests={onShowRequests} />);
}

beforeEach(() => {
  vi.clearAllMocks();
  window.location.hash = "";
  apiMock.getSession.mockResolvedValue(session());
});

describe("a session's own page", () => {
  it("shows every token bucket, the machine, and the id — the things the table had no room for", async () => {
    renderDetail();

    // Four buckets, not two-and-a-tooltip: this page has the width.
    expect(await screen.findByText("4k / 1k / 12k / 640")).toBeInTheDocument();
    expect(screen.getByText(/buildbox/)).toBeInTheDocument();
    expect(screen.getByText("s-1")).toBeInTheDocument();
    // Twice on purpose: the heading and the breadcrumb's leaf.
    expect(screen.getAllByText("port the parser to Rust")).toHaveLength(2);
  });

  it("keeps the two ledgers apart: two currencies are two amounts, never a sum", async () => {
    apiMock.getSession.mockResolvedValue(
      session({
        cost: [
          { currency: "USD", amount: 2.5 },
          { currency: "CNY", amount: 7.25 },
        ],
        unpriced_rows: 3,
      }),
    );
    renderDetail();

    // Adding these would be adding dollars to yuan, and importing the
    // Dashboard's conversion here would be the first conversion outside it.
    expect(await screen.findByText("$2.5 + ¥7.25 (+3 unpriced)")).toBeInTheDocument();
  });

  it("shows a dash for the side that never saw the session, not a zero", async () => {
    // Only the files have it: the gateway routed none of it, and a `0` under
    // "requests" would say it routed nothing — a different statement.
    apiMock.getSession.mockResolvedValue(
      session({ source: "imported", requests: 0, cost: [], turns: 12 }),
    );
    const { unmount } = renderDetail();

    const requests = await screen.findByText(en.dashboard.colRequests);
    expect(requests.parentElement?.textContent).toContain(en.common.none);
    // The files' money is nothing at all rather than a zero: a cost of `0` would
    // read as "this was free".
    const cost = screen.getByText(en.dashboard.sessionCost);
    expect(cost.parentElement?.textContent).toContain(en.common.none);
    unmount();

    // And the other way round: a traffic-only session has no turns to report.
    apiMock.getSession.mockResolvedValue(
      session({ source: "gateway", project: null, machine: null, turns: 0 }),
    );
    renderDetail();
    const turns = await screen.findByText(en.dashboard.sessionTurns);
    expect(turns.parentElement?.textContent).toContain(en.common.none);
  });

  it("tells a file that recorded no tools apart from a session with no file", async () => {
    apiMock.getSession.mockResolvedValue(session({ source: "imported", tool_calls: [] }));
    renderDetail();
    expect(await screen.findByText(en.dashboard.sessionNoTools)).toBeInTheDocument();
    expect(screen.getByText(en.dashboard.sessionNoSkills)).toBeInTheDocument();
  });

  it("lists the calls as a table, in order, one page at a time", async () => {
    // Twelve calls, so the second page is a page rather than the end.
    const made = Array.from({ length: 12 }, (_, i) => ({
      name: `Tool${String(i + 1).padStart(2, "0")}`,
      detail: `did the ${i + 1}th thing`,
    }));
    apiMock.getSession.mockResolvedValue(session({ tool_uses: calls(made) }));
    const user = userEvent.setup();
    renderDetail();

    // Await the body before scoping: the panel exists only once the read lands,
    // and the first paint of this page is the breadcrumb alone.
    // Scoped to the calls panel — the breadcrumb is a list too, and the counts
    // card above has a table-free summary of its own.
    const toolsCard = (await screen.findByText(
      en.dashboard.sessionCalls.replace("{n}", "12"),
    )).closest(".rounded-lg") as HTMLElement;
    const rows = () =>
      within(toolsCard)
        .getAllByRole("row")
        .slice(1) // the header
        .map((r) => r.textContent);

    // The number is the call's place in the session — what a chronology has that
    // a count cannot say.
    expect(rows()).toHaveLength(10);
    // The row is number, when, tool, what — and the number is the call's place
    // in the session, not its index in this list.
    expect(rows()[0]).toBe("109-10 04:41Tool01did the 1th thing");
    expect(rows()[9]).toBe("1009-10 04:50Tool10did the 10th thing");
    expect(within(toolsCard).getByText(en.common.pageIndicator.replace("{page}", "1").replace("{pages}", "2"))).toBeInTheDocument();

    await user.click(within(toolsCard).getByRole("button", { name: en.common.next }));
    expect(rows()).toHaveLength(2);
    expect(rows()[0]).toContain("Tool11");
    expect(rows()[1]).toContain("Tool12");
  });

  it("shows both ends of a long session, and counts what it did not keep", async () => {
    // The shape a 6,000-call session arrives in: the opening, the end, and the
    // number in between that nothing holds.
    apiMock.getSession.mockResolvedValue(
      session({
        tool_uses: calls(
          [
            { name: "Read", detail: "the first thing it read" },
            { name: "Bash", detail: "the second thing it ran" },
          ],
          [{ name: "Write", detail: "the last thing it wrote" }],
          5_733,
        ),
      }),
    );
    renderDetail();

    const panel = (await screen.findByText(
      en.dashboard.sessionCalls.replace("{n}", "3"),
    )).closest(".rounded-lg") as HTMLElement;
    const rows = within(panel)
      .getAllByRole("row")
      .slice(1)
      .map((r) => r.textContent);

    // Head first…
    expect(rows[0]).toContain("the first thing it read");
    // …then the gap, said out loud rather than left to look like a jump…
    expect(rows[2]).toContain(
      en.dashboard.sessionCallsSkipped.replace("{n}", "5,733"),
    );
    // …then the tail, numbered where it actually sits in the session.
    expect(rows[3]).toContain("the last thing it wrote");
    expect(rows[3]).toContain(String(2 + 5_733 + 1));
  });

  it("shows the files' own tokens when both ledgers have the session", async () => {
    // `both`: the merged token fields are the gateway's, so the files' total has
    // to come from its own field or the "two ledgers" columns show one of them
    // missing a number.
    apiMock.getSession.mockResolvedValue(
      session({
        source: "both",
        input_tokens: 10,
        output_tokens: 2,
        file_tokens: { input: 900, output: 100, cache_read: 4_000, cache_creation: 0 },
      }),
    );
    const { unmount } = renderDetail();
    const files = (await screen.findByText(en.dashboard.sessionFromFiles)).closest(
      ".rounded-lg",
    ) as HTMLElement;
    expect(files.textContent).toContain("900 / 100 / 4k / 0");
    unmount();

    // An imported-only session's merged numbers *are* the files': printing them
    // twice would read as two measurements of one thing.
    apiMock.getSession.mockResolvedValue(
      session({
        source: "imported",
        input_tokens: 900,
        output_tokens: 100,
        file_tokens: { input: 900, output: 100, cache_read: 4_000, cache_creation: 0 },
      }),
    );
    renderDetail();
    const files2 = (await screen.findByText(en.dashboard.sessionFromFiles)).closest(
      ".rounded-lg",
    ) as HTMLElement;
    expect(files2.textContent).not.toContain("900 / 100 / 4k / 0");
  });

  it("puts the calls in their own panel, last on the page", async () => {
    apiMock.getSession.mockResolvedValue(
      session({ tool_uses: calls([{ name: "Bash", detail: "ls" }]) }),
    );
    renderDetail();

    // Await the body first: the first paint of this page is the breadcrumb alone.
    await screen.findByText(en.dashboard.sessionCalls.replace("{n}", "1"));
    // The counts card names the tools; the panel below names the calls, and it is
    // the last thing on the page — after the action, which belongs with the
    // summary rather than under a hundred rows of detail.
    const panels = [...document.querySelectorAll(".rounded-lg")].map((el) =>
      el.querySelector("div")?.textContent,
    );
    const callsPanel = panels.findIndex((title) =>
      title?.startsWith(en.dashboard.sessionCalls.replace("{n}", "1")),
    );
    expect(callsPanel).toBe(panels.length - 1);
    expect(panels).toContain(en.dashboard.sessionTools);
  });

  it("shows a call with nothing to summarise, because it happened", async () => {
    apiMock.getSession.mockResolvedValue(
      session({ tool_uses: calls([{ name: "ExitPlanMode", detail: "" }]) }),
    );
    renderDetail();
    const toolsCard = (await screen.findByText(
      en.dashboard.sessionCalls.replace("{n}", "1"),
    )).closest(".rounded-lg") as HTMLElement;
    const row = within(toolsCard).getAllByRole("row")[1];
    expect(row.textContent).toContain("ExitPlanMode");
    // A call with nothing to summarise still appears: it happened.
    expect(row.textContent).toContain(en.common.none);
  });

  it("orders tools by use, caps the list, and says what it left out", async () => {
    const many = Array.from({ length: 12 }, (_, i) => ({
      name: `Tool${String(i + 1).padStart(2, "0")}`,
      count: 12 - i,
    }));
    // The wire order is the file's order, which is not an order a reader wants.
    apiMock.getSession.mockResolvedValue(
      session({ tool_calls: [...many].reverse(), skills: [{ name: "Skill", count: 2 }] }),
    );
    renderDetail();

    await screen.findByText(en.dashboard.sessionTools);
    expect(screen.getByText("Tool01")).toBeInTheDocument();
    expect(screen.getByText("Tool10")).toBeInTheDocument();
    // Two left out, and the page says so rather than quietly ending at ten.
    expect(screen.queryByText("Tool11")).toBeNull();
    expect(
      screen.getByText(en.dashboard.sessionMore.replace("{n}", "2")),
    ).toBeInTheDocument();
    expect(screen.getByText("Skill")).toBeInTheDocument();
  });

  it("says a traffic-only session has no tools, and offers the way to its requests", async () => {
    apiMock.getSession.mockResolvedValue(
      session({ source: "gateway", project: null, turns: 0, tool_calls: [] }),
    );
    const user = userEvent.setup();
    renderDetail();

    // No file ever saw it, so there are no tools — and the page says `—` rather
    // than the files' own "nothing recorded here", which would be a claim about
    // a file that does not exist.
    const tools = await screen.findByText(en.dashboard.sessionTools);
    expect(tools.parentElement?.parentElement?.textContent).toContain(en.common.none);

    await user.click(screen.getByRole("button", { name: en.dashboard.sessionShowRequests }));
    expect(onShowRequests).toHaveBeenCalledWith("s-1");
  });

  it("keeps the breadcrumb in every state, and it goes back to the Dashboard", async () => {
    const user = userEvent.setup();
    const { unmount } = renderDetail();
    await screen.findByRole("heading", { name: "port the parser to Rust" });

    // The root's label is the nav's own string, so the two cannot drift.
    const root = () => screen.getByRole("button", { name: en.app.nav.dashboard });
    await user.click(root());
    expect(window.location.hash).toBe("#dashboard");
    unmount();

    // Still there while the read is in flight…
    apiMock.getSession.mockReturnValue(new Promise(() => {}));
    const pending = renderDetail();
    expect(root()).toBeInTheDocument();
    pending.unmount();

    // …and when the session is gone, where it is the only way out.
    apiMock.getSession.mockResolvedValue(null);
    renderDetail();
    expect(await screen.findByText(en.dashboard.sessionGone)).toBeInTheDocument();
    expect(root()).toBeInTheDocument();
  });

  it("tells a session that is gone from a read that failed", async () => {
    apiMock.getSession.mockRejectedValue(new Error("gateway unreachable"));
    const { unmount } = renderDetail();

    // A failure is a failure: it says so and offers a retry.
    expect(await screen.findByText(en.common.loadFailed)).toBeInTheDocument();
    unmount();

    // `null` is an answer, not an error — the page says the session is gone
    // rather than claiming it could not read the screen.
    apiMock.getSession.mockResolvedValue(null);
    renderDetail();
    expect(await screen.findByText(en.dashboard.sessionGone)).toBeInTheDocument();
    expect(screen.queryByText(en.common.loadFailed)).toBeNull();
  });

  it("reads the session by id, because a refresh or a pasted link arrives with nothing else", async () => {
    renderDetail("s-elsewhere");
    await waitFor(() => expect(apiMock.getSession).toHaveBeenCalledWith("s-elsewhere"));
  });
});
