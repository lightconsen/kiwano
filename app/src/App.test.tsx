// The app shell's own reload: the ⟳ at the right of the status bar.
//
// Its work is tens of milliseconds of local reads — a socket status, a SQLite
// aggregate, and whatever the current screen re-reads — so the spinner is a
// blink and the screen looks identical afterwards (nothing moved, or the data
// would have redrawn). The check is the only thing that separates "refreshed,
// nothing changed" from "the click did nothing", which is what this pins.
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { en } from "@/i18n/en";

import App from "./App";

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
