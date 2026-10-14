// The Dashboard's filter wiring: what the two comboboxes do to the query, and
// what a response is allowed to do back to them.
//
// The numbers themselves are the backend's (`vm::build_dashboard` is asserted in
// Rust, including the invariant the option lists rest on). What lives only here
// is the round trip: the selection has to reach `getDashboard`, the option list
// has to survive it, and a window that no longer has the selected provider has
// to fall back rather than keep filtering on something with no traffic.
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { en } from "@/i18n/en";
import type { DashboardData } from "@/api/types";

import Dashboard from "./Dashboard";

// The page embeds the request-log table, which reads the same filters off its
// props — so it is part of this screen's linkage, not a separate unit.
const { apiMock } = vi.hoisted(() => ({
  apiMock: {
    getCurrencyMeta: vi.fn(),
    getDashboard: vi.fn(),
    listRequestLogs: vi.fn(),
    listSessions: vi.fn(),
    getSession: vi.fn(),
    getRequestLog: vi.fn(),
    exportRequestLogs: vi.fn(),
  },
}));

vi.mock("../api/client", () => ({ api: apiMock }));

/** A dashboard payload. `providers` is the window's active set — the option
    list, which the backend computes independent of the current filter. */
function dashboard(
  over: {
    window?: DashboardData["window"];
    providers?: { id: string; label: string }[];
    requests?: number;
    trend?: DashboardData["trend"];
    byProvider?: DashboardData["by_provider"];
  } = {},
): DashboardData {
  const providers = over.providers ?? [
    { id: "deepseek", label: "DeepSeek" },
    { id: "kimi", label: "Kimi" },
  ];
  return {
    window: over.window ?? "7d",
    requests: over.requests ?? 271,
    requests_delta_pct: 0,
    input_tokens: 1_000,
    cache_read_tokens: 0,
    output_tokens: 500,
    cost: 12.5,
    cost_off_peak: 9.5,
    latency_ms: 900,
    latency_delta_pct: 0,
    trend: over.trend ?? [],
    by_provider: over.byProvider ?? [],
    by_agent: [],
    filter_providers: providers,
    filter_agents: [{ id: "claude", label: "Claude Code" }],
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  apiMock.getCurrencyMeta.mockResolvedValue({ preferred: "CNY", currencies: ["CNY"], exchange_rates: {} });
  apiMock.listRequestLogs.mockResolvedValue({ rows: [], total: 0 });
});

/** One session both ledgers know — enough for the page to render. */
const SESSION = {
  session_id: "s-1",
  source: "both" as const,
  agent: "claude",
  project: "acme-api",
  machine: "buildbox",
  title: "port the parser to Rust",
  started_at: "2026-09-09T20:41:00Z",
  ended_at: "2026-09-09T21:30:00Z",
  requests: 3,
  turns: 7,
  input_tokens: 10,
  output_tokens: 2,
  cache_read_tokens: 0,
  cache_creation_tokens: 0,
  cost: [],
  unpriced_rows: 0,
  tool_calls: [{ name: "Bash", count: 9 }],
  tool_uses: { head: [{ name: "Bash", detail: "cargo test", at: "2026-09-09T20:41:00Z" }], tail: [], skipped: 0 },
  skills: [],
};

describe("a session's own page inside the Dashboard", () => {
  it("replaces the Dashboard rather than floating over it, and comes back where it left", async () => {
    // The Dashboard's own read still runs while the session page is open — its
    // hooks are above the early return — so it has to answer.
    apiMock.getDashboard.mockResolvedValue(dashboard());
    apiMock.listSessions.mockResolvedValue({ rows: [], total: 0 });
    apiMock.getSession.mockResolvedValue(SESSION);
    window.location.hash = "";
    const { rerender } = render(<Dashboard openSessionId="s-1" />);

    // What it shows is a different subject: the panels above it must not render
    // underneath, or the page reads as a detail *of* them.
    expect(await screen.findByRole("heading", { name: "port the parser to Rust" })).toBeInTheDocument();
    expect(screen.queryByText(en.dashboard.usageTrend)).toBeNull();

    // And when the route drops the tail, the Dashboard is back.
    rerender(<Dashboard openSessionId={undefined} />);
    expect(await screen.findByText(en.dashboard.usageTrend)).toBeInTheDocument();
  });
});

describe("the dashboard's filters", () => {
  it("asks for the chosen provider and keeps the option list whole", async () => {
    apiMock.getDashboard
      .mockResolvedValueOnce(dashboard())
      .mockResolvedValueOnce(dashboard({ requests: 199 }));
    const user = userEvent.setup();
    render(<Dashboard />);

    // The mount read is unfiltered.
    await waitFor(() => expect(apiMock.getDashboard).toHaveBeenCalledWith("7d", undefined, undefined));

    await user.click(screen.getByRole("combobox", { name: en.dashboard.filterByProvider }));
    await user.click(await screen.findByRole("option", { name: "DeepSeek" }));
    await waitFor(() =>
      expect(apiMock.getDashboard).toHaveBeenLastCalledWith("7d", "deepseek", undefined),
    );

    // Filtering narrows the *numbers*, not the list of things you can filter
    // by: Kimi is still offered, so the reader can switch to it — and "All
    // providers" is still the way back.
    await user.click(screen.getByRole("combobox", { name: en.dashboard.filterByProvider }));
    expect(await screen.findByRole("option", { name: "Kimi" })).toBeInTheDocument();
    expect(screen.getByRole("option", { name: en.dashboard.allProviders })).toBeInTheDocument();

    // The embedded request-log table follows the same slice — one filter, every
    // panel answering for it.
    await waitFor(() =>
      expect(apiMock.listRequestLogs).toHaveBeenLastCalledWith(
        1,
        expect.any(Number),
        expect.objectContaining({ provider_id: "deepseek" }),
      ),
    );
  });

  it("falls back to All when the new window has no traffic for the selection", async () => {
    // 7d has both providers; 30d only Kimi. Keeping the selection would leave
    // the reader filtering a window that cannot contain it.
    apiMock.getDashboard
      .mockResolvedValueOnce(dashboard())
      .mockResolvedValueOnce(dashboard())
      .mockResolvedValueOnce(dashboard({ window: "30d", providers: [{ id: "kimi", label: "Kimi" }] }))
      .mockResolvedValue(dashboard({ window: "30d", providers: [{ id: "kimi", label: "Kimi" }] }));
    const user = userEvent.setup();
    render(<Dashboard />);
    await waitFor(() => expect(apiMock.getDashboard).toHaveBeenCalledTimes(1));

    await user.click(screen.getByRole("combobox", { name: en.dashboard.filterByProvider }));
    await user.click(await screen.findByRole("option", { name: "DeepSeek" }));
    await waitFor(() => expect(apiMock.getDashboard).toHaveBeenLastCalledWith("7d", "deepseek", undefined));

    await user.click(screen.getByRole("button", { name: en.dashboard.window30d }));

    // The filtered read of 30d answered without DeepSeek, so the screen resets
    // to All and re-reads — the last word is an unfiltered query.
    await waitFor(() =>
      expect(apiMock.getDashboard).toHaveBeenLastCalledWith("30d", undefined, undefined),
    );
    expect(
      screen.getByRole("combobox", { name: en.dashboard.filterByProvider }),
    ).toHaveTextContent(en.dashboard.allProviders);
  });
});

describe("the charts", () => {
  /** The named tone the trend claims: the metric and the bucket count are part
      of the chart's accessible name, which is also how it is found here. */
  const trendAria = (buckets: number) =>
    en.dashboard.trendAria
      .replace("{metric}", en.dashboard.trendMetricRequests)
      .replace("{n}", String(buckets));

  /** The painted bars. The trend's rects are the only ones with a corner radius
      — the full-height hit areas behind them are square and unpainted. */
  const bars = (container: HTMLElement) =>
    [...container.querySelectorAll<SVGRectElement>('svg rect[rx="2"]')];

  it("scales each bar to its bucket, over the window's maximum", async () => {
    apiMock.getDashboard.mockResolvedValue(
      dashboard({
        trend: [
          { date: "09-01", day: "2026-09-01", requests: 10, tokens: 100, cost: 1 },
          { date: "09-02", day: "2026-09-02", requests: 20, tokens: 200, cost: 2 },
          { date: "09-03", day: "2026-09-03", requests: 40, tokens: 400, cost: 4 },
        ],
      }),
    );
    const { container } = render(<Dashboard />);

    // 22 → 112 is the plot's 90px band: the tallest bucket fills it, the others
    // take their share of it (the SVG scales the whole thing to the card).
    const svg = await screen.findByRole("img", { name: trendAria(3) });
    expect(bars(container).map((b) => b.getAttribute("height"))).toEqual([
      "22.5",
      "45",
      "90",
    ]);
    // …and the axis is labelled from the same maximum, not from a fixed scale.
    expect(within(svg).getByText("40")).toBeInTheDocument();
    expect(within(svg).getByText("20")).toBeInTheDocument();
    expect(within(svg).getByText("0")).toBeInTheDocument();
    expect(within(svg).getByText("09-01")).toBeInTheDocument();
  });

  it("thins the axis labels so a 24-hour window does not smear", async () => {
    const hourly = Array.from({ length: 24 }, (_, h) => ({
      date: `${String(h).padStart(2, "0")}:00`,
      day: "2026-10-14",
      requests: 10 + h,
      tokens: 1_000,
      cost: 0.5,
    }));
    apiMock.getDashboard.mockResolvedValue(dashboard({ window: "today", trend: hourly }));
    render(<Dashboard />);

    const svg = await screen.findByRole("img", { name: trendAria(24) });
    const labels = [...svg.querySelectorAll("text")].map((t) => t.textContent);
    // Every third hour — eight labels for 24 buckets, and the ones between them
    // are left out rather than printed over each other.
    expect(labels.filter((l) => l?.endsWith(":00"))).toHaveLength(8);
    expect(labels).toContain("00:00");
    expect(labels).toContain("03:00");
    expect(labels).not.toContain("01:00");
  });

  it("draws an empty window without inventing geometry", async () => {
    // The empty database: max is 0, and every height would be 0/0. The chart
    // still has to be a valid SVG — an invalid attribute is a mark the engine
    // drops silently, which reads as "no traffic" either way only by luck.
    apiMock.getDashboard.mockResolvedValue(dashboard());
    const { container } = render(<Dashboard />);

    await screen.findByRole("img", { name: trendAria(0) });
    expect(bars(container)).toHaveLength(0);
    expect(container.innerHTML).not.toContain("NaN");
  });

  it("reads out the slice the legend points at", async () => {
    apiMock.getDashboard.mockResolvedValue(
      dashboard({
        byProvider: [
          {
            id: "deepseek",
            name: "DeepSeek",
            color: "#4D6BFE",
            requests: 731,
            pct: 62,
            cost: 28.6,
            cost_off_peak: 25.5,
          },
          {
            id: "kimi",
            name: "Kimi",
            color: "#555555",
            requests: 271,
            pct: 23,
            cost: 10.9,
            cost_off_peak: 10.9,
          },
        ],
      }),
    );
    render(<Dashboard />);

    const donut = await screen.findByRole("img", {
      name: en.dashboard.donutAria.replace("{n}", "1002"),
    });
    // Nothing pointed at: the hole answers for the whole window.
    expect(within(donut).getByText("1002")).toBeInTheDocument();
    expect(within(donut).getByText(en.dashboard.donutCenterLabel)).toBeInTheDocument();

    // Focusing a legend row moves the readout onto its slice — the keyboard's
    // way in, since the ring is 15px thick and not a target to ask anyone to hit.
    const row = screen.getByText("DeepSeek").closest("[tabindex]") as HTMLElement;
    expect(row, "legend rows are focusable").not.toBeNull();
    fireEvent.focus(row);
    expect(within(donut).getByText("731")).toBeInTheDocument();
    expect(within(donut).getByText("DeepSeek")).toBeInTheDocument();

    // The legend carries its own numbers, independent of the hover: share and
    // what that share cost.
    expect(screen.getByText(/62%/)).toBeInTheDocument();
  });
});

describe("the activity grid", () => {
  /** The layout switch, which is what puts the grid on screen. Waits for the
      card to exist first: the page renders a loading state until the daemon
      answers, and the switch is inside it. */
  const toGrid = async (user: ReturnType<typeof userEvent.setup>) =>
    user.click(await screen.findByRole("button", { name: en.dashboard.layoutGrid }));

  const cells = (container: HTMLElement) =>
    [...container.querySelectorAll<HTMLElement>("[data-state]")];
  const states = (container: HTMLElement) => cells(container).map((c) => c.dataset.state);

  /** One bucket per day, `n` days running back from `from`. */
  const daily = (from: string, counts: number[]) =>
    counts.map((requests, i) => {
      const d = new Date(`${from}T12:00:00Z`);
      d.setUTCDate(d.getUTCDate() + i);
      return {
        date: `${String(d.getUTCMonth() + 1).padStart(2, "0")}-${String(d.getUTCDate()).padStart(2, "0")}`,
        day: d.toISOString().slice(0, 10),
        requests,
        tokens: requests * 10,
        cost: requests / 10,
      };
    });

  it("draws one cell per bucket, and shades them against the window's own range", async () => {
    const user = userEvent.setup();
    // Weekday-aligned from a Monday, so the leading blanks are none and the
    // rows can be read off the sequence.
    apiMock.getDashboard.mockResolvedValue(
      dashboard({ window: "7d", trend: daily("2026-10-05", [0, 10, 20, 30, 40, 50, 60]) }),
    );
    const { container } = render(<Dashboard />);
    await toGrid(user);

    expect(states(container)).toHaveLength(7);
    // Zero is its own state — "nothing happened" is not "a little happened".
    expect(states(container)[0]).toBe("none");
    // The rest ramp over the *non-zero* buckets, so an idle day does not flatten
    // the scale.
    expect(states(container).slice(1)).toEqual([
      "low",
      "low",
      "medium",
      "medium",
      "high",
      "high",
    ]);
  });

  it("shows an empty window as empty, and says so in the legend", async () => {
    const user = userEvent.setup();
    apiMock.getDashboard.mockResolvedValue(
      dashboard({ window: "7d", trend: daily("2026-10-05", [0, 0, 0, 0, 0, 0, 0]) }),
    );
    const { container } = render(<Dashboard />);
    await toGrid(user);

    expect(states(container).every((s) => s === "none")).toBe(true);
    // Every step is on screen with its count, so the reading does not depend on
    // telling the shades apart.
    expect(screen.getByText(`${en.dashboard.activityLegendNone} 7`)).toBeInTheDocument();
    expect(container.innerHTML).not.toContain("NaN");
  });

  it("falls back to a single row for hours, and marks the hours yet to come", async () => {
    const user = userEvent.setup();
    const today = todayKey();
    const hourly = Array.from({ length: 24 }, (_, h) => ({
      date: `${String(h).padStart(2, "0")}:00`,
      day: today,
      requests: h,
      tokens: h * 10,
      cost: h / 10,
    }));
    apiMock.getDashboard.mockResolvedValue(dashboard({ window: "today", trend: hourly }));
    const { container } = render(<Dashboard />);
    await toGrid(user);

    // A weekday means nothing for an hour, so the calendar is not used: the grid
    // is one row in the order the buckets come in.
    // Every window lays out the same way — seven cells to a row, filling left
    // to right — so an hour is a cell like any other.
    const grid = container.querySelector<HTMLElement>("[role='img'] div.grid")!;
    expect(grid.style.gridTemplateColumns).toMatch(/^repeat\(7, \d+px\)$/);

    // The hours after now are not "no activity" — they have not happened yet,
    // and the grid says which is which rather than reporting the future.
    const nowHour = new Date().getHours();
    const shown = states(container);
    expect(shown.filter((s) => s === "future")).toHaveLength(23 - nowHour);
    if (nowHour > 0) expect(shown[0]).not.toBe("future");
  });

  it("switches metric without changing the layout, and the caption names the bucket", async () => {
    const user = userEvent.setup();
    apiMock.getDashboard.mockResolvedValue(
      dashboard({ window: "7d", trend: daily("2026-10-05", [1, 2, 3, 4, 5, 6, 7]) }),
    );
    const { container } = render(<Dashboard />);
    await toGrid(user);

    expect(cells(container)).toHaveLength(7);
    expect(
      screen.getByText(
        en.dashboard.activityCaption
          .replace("{unit}", en.dashboard.activityCellDay)
          .replace("{metric}", en.dashboard.trendMetricRequests),
      ),
    ).toBeInTheDocument();

    // Cost is the third series: the same buckets, read as money, in the display
    // currency the daemon already converted them into.
    await user.click(screen.getByRole("button", { name: en.dashboard.metricCost }));
    expect(cells(container)).toHaveLength(7);
    expect(
      screen.getByText(
        en.dashboard.activityCaption
          .replace("{unit}", en.dashboard.activityCellDay)
          .replace("{metric}", en.dashboard.trendMetricCost),
      ),
    ).toBeInTheDocument();
    // The grid is still the grid — the metric switch is not a layout switch.
    expect(container.querySelector("svg rect[rx='2']")).toBeNull();
  });

  it("fills each row to seven cells, then starts the next", async () => {
    const user = userEvent.setup();
    apiMock.getDashboard.mockResolvedValue(
      dashboard({
        window: "30d",
        trend: daily("2026-09-14", Array.from({ length: 30 }, (_, i) => i + 1)),
      }),
    );
    const { container } = render(<Dashboard />);
    await toGrid(user);

    const grid = container.querySelector<HTMLElement>("[role='img'] div.grid")!;
    // Seven to a row, in the order the buckets happened: 30 days is four full
    // rows and a two-cell tail. No weekday alignment, no leading blanks — a
    // blank would be a cell that is not a bucket.
    expect(grid.style.gridTemplateColumns).toMatch(/^repeat\(7, \d+px\)$/);
    expect(grid.children).toHaveLength(30);
    expect(cells(container)).toHaveLength(30);
    expect(cells(container)[0]).toBe(grid.children[0]);
  });

  it("is reachable by keyboard, and the arrows follow the grid's own axes", async () => {
    const user = userEvent.setup();
    apiMock.getDashboard.mockResolvedValue(
      dashboard({ window: "30d", trend: daily("2026-09-14", Array.from({ length: 30 }, (_, i) => i + 1)) }),
    );
    const { container } = render(<Dashboard />);
    await toGrid(user);

    const grid = container.querySelector<HTMLElement>("[role='img']")!;
    fireEvent.focus(grid);
    // Focusing seeds the readout on the first bucket, like the trend chart's.
    expect(screen.getByRole("status")).toHaveTextContent("1 requests");

    // The arrows walk the grid as drawn: one cell along the row, seven down.
    fireEvent.keyDown(grid, { key: "ArrowRight" });
    expect(screen.getByRole("status")).toHaveTextContent("2 requests");
    fireEvent.keyDown(grid, { key: "ArrowDown" });
    expect(screen.getByRole("status")).toHaveTextContent("9 requests");
    fireEvent.keyDown(grid, { key: "ArrowLeft" });
    expect(screen.getByRole("status")).toHaveTextContent("8 requests");
    // Up is a row, so it lands one row above where right left off.
    fireEvent.keyDown(grid, { key: "ArrowUp" });
    expect(screen.getByRole("status")).toHaveTextContent("1 requests");
    // …and the ends hold: the first cell is a wall, not a wrap-around.
    fireEvent.keyDown(grid, { key: "ArrowUp" });
    fireEvent.keyDown(grid, { key: "ArrowLeft" });
    expect(screen.getByRole("status")).toHaveTextContent("1 requests");
  });
});

/** Today, as the trend's own key spells it. */
function todayKey(): string {
  const d = new Date();
  const m = String(d.getMonth() + 1).padStart(2, "0");
  return `${d.getFullYear()}-${m}-${String(d.getDate()).padStart(2, "0")}`;
}
