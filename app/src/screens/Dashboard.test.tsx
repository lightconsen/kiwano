// The Dashboard's filter wiring: what the two comboboxes do to the query, and
// what a response is allowed to do back to them.
//
// The numbers themselves are the backend's (`vm::build_dashboard` is asserted in
// Rust, including the invariant the option lists rest on). What lives only here
// is the round trip: the selection has to reach `getDashboard`, the option list
// has to survive it, and a window that no longer has the selected provider has
// to fall back rather than keep filtering on something with no traffic.
import { render, screen, waitFor } from "@testing-library/react";
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
    trend: [],
    by_provider: [],
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
