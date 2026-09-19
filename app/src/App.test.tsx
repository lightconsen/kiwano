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
import { act, render, screen, waitFor } from "@testing-library/react";
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
});

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
