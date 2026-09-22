// The app shell's own reload: the ⟳ at the right of the status bar — and the
// live numbers the gateway pushes at it.
//
// The ⟳'s work is tens of milliseconds of local reads — a socket status, a
// SQLite aggregate, and whatever the current screen re-reads — so the spinner is
// a blink and the screen looks identical afterwards (nothing moved, or the data
// would have redrawn). The check is the only thing that separates "refreshed,
// nothing changed" from "the click did nothing", which is what this pins.
//
// The tick is the same read without the user: the gateway says a request was
// recorded, and what is on screen re-reads.
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { en } from "@/i18n/en";

import App from "./App";

// The event bridge is a no-op outside the webview (`lib/updateEvents` reads this
// flag when it loads), so the file has to look like one before that module is
// imported — which is what `vi.hoisted` runs before. Without it every
// subscription in the shell silently unsubscribes from nothing.
vi.hoisted(() => {
  (window as unknown as { __TAURI_INTERNALS__: unknown }).__TAURI_INTERNALS__ = {};
});

/** The listeners the shell registered, by event name. */
const listeners = new Map<string, () => void>();

vi.mock("@tauri-apps/api/event", () => ({
  listen: (name: string, cb: () => void) => {
    listeners.set(name, cb);
    return Promise.resolve(() => listeners.delete(name));
  },
}));

const { apiMock } = vi.hoisted(() => ({
  apiMock: {
    getGatewayStatus: vi.fn(),
    getFooterStats: vi.fn(),
    getSettings: vi.fn(),
    getCurrencyMeta: vi.fn(),
    updateSettings: vi.fn(),
    listProviders: vi.fn(),
    getAgentRoutes: vi.fn(),
    detectAgents: vi.fn(),
    probeAgentVersions: vi.fn(),
    // The update banner the shell renders reads this on mount.
    getPendingUpdate: vi.fn(),
    // The credential banner polls this on mount, and the click-through opens
    // the Dashboard's request-log dialog for the finding's row.
    checkCredentialFinding: vi.fn(),
    ackCredentialFinding: vi.fn(),
    // The Dashboard screen the banner navigates to reads all of these.
    getDashboard: vi.fn(),
    listRequestLogs: vi.fn(),
    getRequestLog: vi.fn(),
  },
}));

vi.mock("./api/client", () => ({ api: apiMock }));

beforeEach(() => {
  vi.clearAllMocks();
  listeners.clear();
  apiMock.getGatewayStatus.mockResolvedValue({ running: true, port: 8317, blocked: [] });
  apiMock.getFooterStats.mockResolvedValue({
    today_requests: 1,
    today_tokens: 10,
    hub_synced: true,
    version: "0.0.0",
  });
  apiMock.getSettings.mockResolvedValue({
    theme: "dark",
    language: "en",
    preferred_currency: "CNY",
    custom_agents: [],
    takeovers: [],
    tz_offset_minutes: 0,
    hub_url: "https://hub.example",
  });
  apiMock.getCurrencyMeta.mockResolvedValue({
    preferred: "CNY",
    currencies: ["CNY", "USD"],
    exchange_rates: {},
  });
  apiMock.updateSettings.mockResolvedValue({});
  apiMock.listProviders.mockResolvedValue([]);
  apiMock.getAgentRoutes.mockResolvedValue([]);
  apiMock.detectAgents.mockResolvedValue([]);
  apiMock.probeAgentVersions.mockResolvedValue([]);
  apiMock.getPendingUpdate.mockResolvedValue(null);
  apiMock.checkCredentialFinding.mockResolvedValue(null);
  apiMock.ackCredentialFinding.mockResolvedValue(undefined);
  apiMock.getDashboard.mockResolvedValue({
    requests: 0,
    requests_delta_pct: null,
    input_tokens: 0,
    output_tokens: 0,
    cache_read_tokens: 0,
    cache_creation_tokens: 0,
    reasoning_tokens: 0,
    latency_ms: 0,
    latency_delta_pct: null,
    cost: 0,
    cost_off_peak: 0,
    trend: [],
    by_provider: [],
    by_agent: [],
    filter_providers: [],
    filter_agents: [],
  });
  apiMock.listRequestLogs.mockResolvedValue({ rows: [], total: 0 });
  apiMock.getRequestLog.mockResolvedValue(null);
});

/** ONE_SHAPE: the minimal log row the banner click opens. */
function findingRow(id: number) {
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
    request_notes: "dlp: github-token ×1",
  };
}

describe("the status bar's reload", () => {
  it("flashes a check once the reads land, then goes back to the glyph", async () => {
    const user = userEvent.setup();
    render(<App />);
    const button = await screen.findByRole("button", { name: en.common.refresh });

    // At rest it is the reload glyph — lucide names the icon on the svg, which
    // is the only handle the button has (its label is the action, not the state).
    expect(button.querySelector("svg.lucide-refresh-cw")).not.toBeNull();

    await user.click(button);

    // The acknowledgement, once every read has landed…
    await waitFor(() => expect(button.querySelector("svg.lucide-check")).not.toBeNull());
    // …and it is a flash, not a mode: it clears itself.
    await waitFor(
      () => expect(button.querySelector("svg.lucide-refresh-cw")).not.toBeNull(),
      { timeout: 3000 },
    );
  });
});

describe("the gateway's live numbers", () => {
  it("re-reads what is on screen when a request has been recorded", async () => {
    render(<App />);

    // The shell subscribes on mount and reads the totals once by itself.
    await waitFor(() => expect(listeners.has("usage-changed")).toBe(true));
    await waitFor(() => expect(apiMock.getFooterStats).toHaveBeenCalledTimes(1));

    // One tick per recorded request: nothing polls for the numbers, the gateway
    // says when there are new ones.
    act(() => listeners.get("usage-changed")!());

    await waitFor(() => expect(apiMock.getFooterStats).toHaveBeenCalledTimes(2));
  });
});

describe("the credential banner's click-through", () => {
  it("opens the request-log detail on the Dashboard it navigates to", async () => {
    window.location.hash = "";
    const user = userEvent.setup();

    apiMock.checkCredentialFinding.mockResolvedValue(findingRow(41));
    apiMock.getRequestLog.mockResolvedValue({
      ...findingRow(41),
      request_body: null,
      response_body: null,
    });
    render(<App />);

    // The banner shows the finding; the click acks and deep-links the row.
    await screen.findByText(en.app.credentialBannerTitle);
    await user.click(screen.getByRole("button", { name: en.app.credentialBannerCta }));
    expect(apiMock.ackCredentialFinding).toHaveBeenCalledWith(41);

    // The Dashboard the route lands on asks for the row by id and opens the
    // detail dialog: the whole reason the deep-link exists.
    await waitFor(() => expect(apiMock.getRequestLog).toHaveBeenCalledWith(41));
    const dialog = await screen.findByRole("dialog");
    expect(
      within(dialog).getByText(new RegExp(en.logs.logTitle.replace("{id}", "41"))),
    ).toBeInTheDocument();

    // The link is spent: the hash rewrites to plain Dashboard once the row is
    // open, and the dialog survives its own deep-link being consumed.
    await waitFor(() => expect(window.location.hash).toBe("#dashboard"));
    expect(screen.getByRole("dialog")).toBeInTheDocument();
  });
});
