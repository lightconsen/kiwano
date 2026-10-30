// The request-log table's own behaviours: what the pager asks for, what a
// status tab does to the page, and the three ways the export can end.
//
// It is embedded in the Dashboard too (`<RequestLogs agent providerId />`), so
// the filter narrowing is covered there; these are the parts that only exist
// when the table is used for itself. The export is the one that leaves the
// machine, which is why all three outcomes are pinned — including the two that
// write nothing.
import { useState } from "react";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { en } from "@/i18n/en";
import type { RequestLogEntry, Session } from "@/api/types";

import RequestLogs, { type Tab } from "./RequestLogs";

const { apiMock, saveMock, openSession, openLog } = vi.hoisted(() => ({
  openSession: vi.fn(),
  openLog: vi.fn(),
  apiMock: {
    listRequestLogs: vi.fn(),
    getRequestLog: vi.fn(),
    exportRequestLogs: vi.fn(),
    listSessions: vi.fn(),
    getSession: vi.fn(),
  },
  saveMock: vi.fn(),
}));

vi.mock("../api/client", () => ({ api: apiMock }));
// The file dialog is a Tauri plugin, called straight from the screen; cancelling
// it is `resolve(null)`, which is a decision and not a failure.
vi.mock("@tauri-apps/plugin-dialog", () => ({ save: saveMock }));

/** ONE_SHAPE: a log row with nothing exotic about it. */
function entry(id: number): RequestLogEntry {
  return {
    id,
    ts: "2026-09-14T10:00:00Z",
    method: "POST",
    path: "/v1/messages",
    query: null,
    agent: "claude",
    attribution: "key",
    provider_id: "deepseek",
    model: "deepseek-chat",
    status_code: 200,
    error_kind: null,
    error_message: null,
    session_id: null,
    is_streaming: false,
    input_tokens: 1_000,
    output_tokens: 200,
    cache_read_tokens: 0,
    cache_creation_tokens: 0,
    reasoning_tokens: 0,
    usage_missing: false,
    latency_ms: 900,
    first_token_ms: 300,
    request_headers: null,
    response_headers: null,
    request_size: 0,
    response_size: 0,
    truncated: false,
    request_notes: null,
  };
}

/** A page of rows, `total` in the database. */
function page(ids: number[], total: number) {
  return { rows: ids.map(entry), total };
}

/** The card with its two view-state values supplied the way the Dashboard
 *  supplies them.
 *
 *  They are controlled props now (the Dashboard owns them so a trip into a
 *  session's page can come back to the tab it left), and a test that clicked the
 *  Sessions tab has to keep working — so the harness is that owner, in ten
 *  lines, rather than a stub that does nothing. `onOpenSession` is a spy: the
 *  row click leaves this card now, and what it leaves to is the Dashboard's
 *  business (see `Dashboard.test.tsx`). */
function Harness(props: Partial<Parameters<typeof RequestLogs>[0]> = {}) {
  const [tab, setTab] = useState<Tab>("logs");
  const [sessionFilter, setSessionFilter] = useState<string | null>(null);
  return (
    <RequestLogs
      tab={tab}
      onTab={setTab}
      sessionFilter={sessionFilter}
      onSessionFilter={setSessionFilter}
      onOpenSession={openSession}
      onOpenLog={openLog}
      managedAgents={[]}
      {...props}
    />
  );
}

async function openExportDialog() {
  const user = userEvent.setup();
  render(<Harness />);
  // The toolbar's button is disabled until the first page has answered, and a
  // click on a disabled button is swallowed — so wait for it to be enabled.
  await waitFor(() =>
    expect(screen.getByRole("button", { name: en.logs.exportCsv })).toBeEnabled(),
  );
  await user.click(screen.getByRole("button", { name: en.logs.exportCsv }));
  return { user, dialog: await screen.findByRole("dialog") };
}

/** The tooltip a session row carries, by id.
 *
 * The id is no longer a column — at the window's 900px minimum this card cannot
 * hold ten of them without cutting the right-hand ones off — so it travels on the
 * row, which is also how a test finds a particular row now. */
function rowTitle(id: string): string {
  return en.logs.sessionRowTitle.replace("{id}", id);
}

/** ONE_SHAPE: a session the gateway routed *and* the files recorded — the
    ordinary state after a takeover, and the one whose numbers come from both
    sides. */
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

beforeEach(() => {
  vi.clearAllMocks();
  apiMock.listRequestLogs.mockResolvedValue(page([1, 2, 3], 25));
  apiMock.listSessions.mockResolvedValue({ rows: [], total: 0 });
  saveMock.mockResolvedValue("/tmp/kiwano-logs.csv");
  apiMock.exportRequestLogs.mockResolvedValue({ rows_written: 3, truncated: false });
});

describe("the request log table", () => {
  it("pages through the query and returns to the first page when the slice changes", async () => {
    const user = userEvent.setup();
    render(<Harness />);
    await waitFor(() => expect(apiMock.listRequestLogs).toHaveBeenCalledWith(1, 10, {}));

    // 25 rows at 10 a page: three pages, and Next is the way through them.
    expect(screen.getByText(en.common.pageIndicator.replace("{page}", "1").replace("{pages}", "3"))).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: en.common.next }));
    await waitFor(() => expect(apiMock.listRequestLogs).toHaveBeenLastCalledWith(2, 10, {}));

    // A status tab changes *which rows there are*, not which page of them is on
    // screen — so the page goes back to the start rather than asking for page 2
    // of a set nobody has seen page 1 of.
    await user.click(screen.getByRole("button", { name: en.logs.filterError }));
    await waitFor(() =>
      expect(apiMock.listRequestLogs).toHaveBeenLastCalledWith(1, 10, { status: "error" }),
    );
  });

  it("draws every row one line tall, however long the ids are", async () => {
    // The three columns that hold user-chosen ids have no upper bound: an agent
    // id, a provider id whose suffix tells two of the same apart, a model id.
    // Left to size themselves, they wrapped at a hyphen and the table came out
    // ragged — a different height per row. `table-fixed` with a truncated cell
    // is the fix, and the full value stays on the cell's title.
    apiMock.listRequestLogs.mockResolvedValue({
      rows: [
        {
          ...entry(1),
          agent: "hello-92fc46-longer-agent-name",
          provider_id: "command-code-a4fafe-with-a-really-long-suffix",
          model: "deepseek/deepseek-v4.1-preview-2026-10-30",
        },
      ],
      total: 1,
    });
    const { container } = render(<Harness />);

    const table = container.querySelector("table")!;
    expect(table.className).toContain("table-fixed");

    const row = (await screen.findByText(en.logs.colAgent)).closest("table")!
      .querySelector("tbody tr")!;
    const cells = [...row.querySelectorAll("td")];
    // Capped, not wrapped: the cell clips and says so on hover.
    for (const [i, value] of [
      [1, "hello-92fc46-longer-agent-name"],
      [2, "command-code-a4fafe-with-a-really-long-suffix"],
      [3, "deepseek/deepseek-v4.1-preview-2026-10-30"],
    ] as const) {
      expect(cells[i].className).toContain("truncate");
      expect(cells[i].getAttribute("title")).toBe(value);
    }
    // Every other cell is nowrap, so no column can grow a second line either.
    for (const td of cells) {
      expect(
        td.className.includes("whitespace-nowrap") || td.className.includes("truncate"),
        td.className,
      ).toBe(true);
    }
  });

  it("writes nothing when the file dialog is cancelled", async () => {
    saveMock.mockResolvedValue(null);
    const { user, dialog } = await openExportDialog();

    await user.click(within(dialog).getByRole("button", { name: en.logs.exportCsv }));

    await waitFor(() => expect(saveMock).toHaveBeenCalled());
    expect(apiMock.exportRequestLogs).not.toHaveBeenCalled();
    // No notice and no error: cancelling is not an outcome worth reporting.
    expect(within(dialog).queryByText(/Exported/)).toBeNull();
  });

  it("exports the chosen path", async () => {
    const { user, dialog } = await openExportDialog();

    // There is no bodies switch to flip: the file always carries them, so the
    // call is the path and the filter and nothing else.
    await user.click(within(dialog).getByRole("button", { name: en.logs.exportCsv }));

    await waitFor(() =>
      expect(apiMock.exportRequestLogs).toHaveBeenCalledWith(
        "/tmp/kiwano-logs.csv",
        expect.any(Object),
      ),
    );
    // The dialog closes on success, and the row count is reported where the
    // toolbar can show it.
    expect(await screen.findByText(en.logs.exported.replace("{n}", "3"))).toBeInTheDocument();
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("keeps the dialog open and says so when the export is capped", async () => {
    apiMock.exportRequestLogs.mockResolvedValue({ rows_written: 500_000, truncated: true });
    const { user, dialog } = await openExportDialog();

    await user.click(within(dialog).getByRole("button", { name: en.logs.exportCsv }));

    // A capped file is a partial answer: the reader has to narrow the range and
    // try again, so the dialog stays put with the reason in it. (The toolbar
    // repeats the line — `err` is also rendered there — so this is scoped.)
    expect(
      await within(dialog).findByText(
        en.logs.exportCapped.replace("{n}", (500_000).toLocaleString()),
      ),
    ).toBeInTheDocument();
    expect(screen.getByRole("dialog")).toBeInTheDocument();
  });

  it("badges a row with a credential finding", async () => {
    const user = userEvent.setup();
    const finding = {
      ...entry(7),
      request_notes: "dlp: github-token ×1\nthinking: removed invalid value",
    };
    apiMock.listRequestLogs.mockResolvedValue({
      rows: [finding, { ...entry(8), model: "other-model" }],
      total: 2,
    });
    apiMock.getRequestLog.mockResolvedValue({ ...finding, request_body: null, response_body: null });
    render(<Harness />);

    // The badge rides the status cell of the finding's row only.
    expect(await screen.findByText(en.logs.dlpBadge)).toBeInTheDocument();
    expect(screen.getAllByText(en.logs.dlpBadge)).toHaveLength(1);

    // The row opens the log's own page (the Dashboard owns the navigation);
    // nothing floats over the table.
    await user.click(screen.getByText("deepseek-chat"));
    expect(openLog).toHaveBeenCalledWith(7);
    expect(screen.queryByRole("dialog")).toBeNull();
  });

});

// ── The sessions view (the card's second tab) ──
//
// The window it reads over is the Dashboard's, and these pin the two things
// that would be silent if they broke: that the log table is what opens, and
// that a session row's click actually becomes a filter on it.

describe("the sessions view", () => {
  it("reads nothing until it is opened, then reads over the Dashboard's window", async () => {
    const user = userEvent.setup();
    render(<Harness agent="claude" window="7d" />);

    // The card opens on the audit trail, and a view nobody has asked for must
    // not cost a query.
    await waitFor(() => expect(apiMock.listRequestLogs).toHaveBeenCalled());
    expect(apiMock.listSessions).not.toHaveBeenCalled();

    await user.click(screen.getByRole("tab", { name: en.logs.tabSessions }));
    // The bound is an instant, not a day count: the dashboard's figures above
    // this card are whole local days, and the rolling reading would disagree
    // with them at the day boundary. So: an agent, and a `since` that is this
    // local midnight six days back.
    await waitFor(() => expect(apiMock.listSessions).toHaveBeenCalled());
    const [agent, project, days, since] = apiMock.listSessions.mock.calls[0];
    expect(agent).toBe("claude");
    expect(project).toBeUndefined();
    expect(days).toBeUndefined();
    // A local midnight — the bound the dashboard's own window starts at, and
    // therefore usually not midnight UTC (this file's midnight is +16:00Z on a
    // UTC+8 machine). Six days back, because "7d" counts today as one of them.
    const start = new Date(since as string);
    expect(start.getHours()).toBe(0);
    expect(start.getMinutes()).toBe(0);
    const sixDaysBack = new Date(start);
    sixDaysBack.setDate(sixDaysBack.getDate() + 6);
    const todayLocal = new Date();
    expect(sixDaysBack.toDateString()).toBe(todayLocal.toDateString());
  });

  it("all time sends no bound at all", async () => {
    const user = userEvent.setup();
    render(<Harness window="all" />);
    await user.click(screen.getByRole("tab", { name: en.logs.tabSessions }));
    await waitFor(() => expect(apiMock.listSessions).toHaveBeenCalled());
    expect(apiMock.listSessions.mock.calls[0][3]).toBeUndefined();
  });

  it("hands a row's session to the Dashboard, which owns where it goes", async () => {
    apiMock.listSessions.mockResolvedValue({ rows: [session("s-1"), session("s-2")], total: 2 });
    const user = userEvent.setup();
    render(<Harness window="7d" />);

    await user.click(screen.getByRole("tab", { name: en.logs.tabSessions }));
    await user.click(await screen.findByTitle(rowTitle("s-2")));

    // Opening a row is no longer "filter this table": it is "show me that
    // session", which is the Dashboard's navigation to make. The card reports
    // the id and keeps its own hands off the filter.
    await waitFor(() => expect(openSession).toHaveBeenCalledWith("s-2"));
    expect(apiMock.listRequestLogs).toHaveBeenLastCalledWith(1, 10, expect.not.objectContaining({
      session_id: expect.anything(),
    }));
  });

  it("narrows the table when the filter is set from outside, and the chip is the way back", async () => {
    // The other half of the same contract: the session page's button sets this
    // filter through the Dashboard, and the card has to honour it like any other
    // bound on the query.
    const user = userEvent.setup();
    const outside = vi.fn();
    render(
      <RequestLogs
        window="7d"
        tab="logs"
        onTab={() => {}}
        sessionFilter="s-2"
        onSessionFilter={outside}
        onOpenSession={openSession}
        onOpenLog={openLog}
        managedAgents={[]}
      />,
    );

    await waitFor(() =>
      expect(apiMock.listRequestLogs).toHaveBeenLastCalledWith(
        1,
        10,
        expect.objectContaining({ session_id: "s-2" }),
      ),
    );
    await user.click(screen.getByRole("button", { name: en.logs.clearSessionFilter }));
    expect(outside).toHaveBeenCalledWith(null);
  });

  it("says so when a session has no gateway traffic, rather than looking broken", async () => {
    apiMock.listRequestLogs.mockResolvedValue({ rows: [], total: 0 });
    render(
      <RequestLogs
        tab="logs"
        onTab={() => {}}
        sessionFilter="imported-1"
        onSessionFilter={() => {}}
        onOpenSession={openSession}
        onOpenLog={openLog}
        managedAgents={[]}
      />,
    );

    // An empty result that is *correct* must not read as a broken filter: the
    // session came out of a file, so the gateway has nothing to show for it.
    expect(await screen.findByText(en.logs.sessionNoTraffic)).toBeInTheDocument();
    expect(screen.queryByText(en.logs.empty)).toBeNull();
  });

  it("keeps the two sides apart in the row it draws", async () => {
    apiMock.listSessions.mockResolvedValue({
      rows: [
      session("both-1", {
        source: "both",
        cost: [
          { currency: "USD", amount: 1.5 },
          { currency: "CNY", amount: 7.25 },
        ],
        unpriced_rows: 2,
      }),
      session("imported-1", { source: "imported", requests: 0, cost: [] }),
      session("gateway-1", { source: "gateway", project: null, turns: 0 }),
      ],
      total: 3,
    });
    const user = userEvent.setup();
    render(<Harness window="7d" />);
    await user.click(screen.getByRole("tab", { name: en.logs.tabSessions }));
    await screen.findByTitle(rowTitle("both-1"));

    // The title is the row's leading column — what a reader scans for, and the
    // id never was — and it is shown **whole**: a title is the one field here a
    // reader has to read rather than compare, so it wraps onto as many lines as
    // it needs instead of ellipsising. Rows are as tall as their titles.
    const titleCell = screen.getByText("session both-1");
    expect(titleCell.className).toContain("break-words");
    expect(titleCell.className).not.toContain("truncate");
    // Two currencies are two amounts. Adding them would be adding dollars to
    // yuan, and only the Dashboard converts.
    expect(screen.getByText("$1.5 + ¥7.25 (+2 unpriced)")).toBeInTheDocument();
    // A side that never saw the session shows a dash rather than a zero: the
    // files carry no money, so `0` there would read as "this was free".
    expect(screen.getByText(en.logs.sourceImported)).toBeInTheDocument();
    expect(screen.getByText(en.logs.sourceGateway)).toBeInTheDocument();
  });

  it("fits a narrow window by truncating and by moving detail to tooltips", async () => {
    // The window's minimum is 900px, which leaves this card about 840 for nine
    // columns — so the table is `table-fixed` and the columns that can be are
    // pinned. What a narrow window cannot show is not lost: it is on the row.
    apiMock.listSessions.mockResolvedValue({
      rows: [
        session("s-1", {
          machine: "buildbox",
          input_tokens: 4_210,
          output_tokens: 1_180,
          cache_read_tokens: 12_400,
          cache_creation_tokens: 640,
          cost: [{ currency: "USD", amount: 2.5 }],
          unpriced_rows: 3,
        }),
      ],
      total: 1,
    });
    const user = userEvent.setup();
    render(<Harness window="7d" />);
    await user.click(screen.getByRole("tab", { name: en.logs.tabSessions }));
    const row = await screen.findByTitle(rowTitle("s-1"));

    const table = screen.getByRole("table");
    expect(table.className).toContain("table-fixed");

    // The cell shows the two buckets a reader compares; the other two are named
    // on hover, with their labels — a number without its bucket is not evidence.
    const tokens = row.querySelectorAll("td")[7];
    expect(tokens.textContent).toBe("4k / 1k");
    expect(tokens.getAttribute("title")).toBe(
      en.logs.tokensCellTitle
        .replace("{input}", "4k")
        .replace("{output}", "1k")
        .replace("{read}", "12k")
        .replace("{write}", "640"),
    );

    // The project cell carries the machine, which has no column of its own.
    const project = row.querySelectorAll("td")[3];
    expect(project.getAttribute("title")).toContain("buildbox");

    // Money is never truncated: a two-currency cost does not fit this card and
    // never will, so it wraps. The widths came from measuring the widest value
    // each column really holds at the window's 900px minimum.
    const cost = row.querySelectorAll("td")[8];
    expect(cost.textContent).toBe("$2.5 (+3 unpriced)");
    expect(cost.firstElementChild?.className).toContain("break-words");

    // The same for the agent, whose longest id in the roster is
    // `deepseek-harness` — 114px of text in a column budgeted at 64.
    const agent = row.querySelectorAll("td")[2];
    expect(agent.firstElementChild?.className).toContain("break-words");
  });

  it("pages the list, and counts what is there rather than what is on screen", async () => {
    // Sixty sessions, ten a page — the card's page size, shared with the log
    // table above it.
    const all = Array.from({ length: 60 }, (_, i) => session(`s-${String(i + 1).padStart(2, "0")}`));
    apiMock.listSessions.mockImplementation(
      async (_a, _p, _d, _since, _agents, page: number, pageSize: number) => ({
        rows: all.slice((page - 1) * pageSize, page * pageSize),
        total: all.length,
      }),
    );
    const user = userEvent.setup();
    render(<Harness window="7d" />);
    await user.click(screen.getByRole("tab", { name: en.logs.tabSessions }));

    // The count is the whole list, not this page of it — a header that said "10
    // sessions" over sixty would be a lie the pager immediately contradicts.
    await screen.findByTitle(rowTitle("s-01"));
    expect(screen.getByText(en.logs.sessionsCount.replace("{n}", "60"))).toBeInTheDocument();
    expect(screen.getByText(en.common.pageIndicator.replace("{page}", "1").replace("{pages}", "6"))).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: en.common.next }));
    await waitFor(() => expect(apiMock.listSessions).toHaveBeenLastCalledWith(
      // The fifth argument is `undefined` here because vitest runs with
      // `import.meta.env.DEV` true — the developing case, which asks for the whole
      // ledger. The release case is pinned by the test below.
      undefined, undefined, undefined, expect.any(String), undefined, 2, 10,
    ));
    await screen.findByTitle(rowTitle("s-11"));
    // Page two does not carry page one's rows.
    expect(screen.queryByTitle(rowTitle("s-01"))).toBeNull();
  });

  it("asks for the agents it manages, and says which empty it is showing", async () => {
    // `vi.stubEnv`, because the rule is a build-time constant: the release build
    // is the one that narrows the list to what it manages.
    vi.stubEnv("DEV", false);
    try {
      apiMock.listSessions.mockResolvedValue({ rows: [], total: 0 });
      const user = userEvent.setup();
      // A machine that has taken nothing over: the app sends an empty set (which
      // the daemon reads as "none of them", not "no filter").
      render(<Harness window="7d" managedAgents={[]} />);
      await user.click(screen.getByRole("tab", { name: en.logs.tabSessions }));

      // The set travels with the query, so the daemon pages *within* it — a client
      // that filtered its own page would show ten rows under a pager counting
      // fifty.
      await waitFor(() =>
        expect(apiMock.listSessions).toHaveBeenLastCalledWith(
          undefined,
          undefined,
          undefined,
          expect.any(String),
          [],
          1,
          10,
        ),
      );

      // Nothing managed is a *different* empty from nothing in this window, and it
      // is the one a reader can act on: it says a takeover is what fills the list.
      expect(screen.queryByText(en.logs.sessionsEmpty)).toBeNull();
      expect(screen.getByText(en.logs.sessionsEmptyUnmanaged)).toBeInTheDocument();
    } finally {
      vi.unstubAllEnvs();
    }
  });

  it("says the provider filter does not apply here instead of pretending it does", async () => {
    const user = userEvent.setup();
    render(<Harness providerId="deepseek" window="7d" />);
    await user.click(screen.getByRole("tab", { name: en.logs.tabSessions }));
    expect(
      await screen.findByText(en.logs.sessionsProviderNotApplied),
    ).toBeInTheDocument();
    // Structurally too: the call has no provider to filter by.
    // Seven arguments, and a provider is not one of them: the structural
    // guarantee behind the note the reader is shown.
    expect(apiMock.listSessions.mock.calls[0]).toHaveLength(7);
  });
});
