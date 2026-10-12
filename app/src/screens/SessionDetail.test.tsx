// One session's own page: what it shows, and the rules it has to keep while
// showing it.
//
// The rules are the interesting part — a session's numbers come from two
// ledgers, and this page is the first place with room to say so. So the tests
// are mostly about what must *not* happen: nothing summed, no zero where a side
// never looked, no money converted.
import { render, screen, waitFor } from "@testing-library/react";
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
    skills: [],
    ...over,
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
