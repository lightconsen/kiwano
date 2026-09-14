// Settings' takeover switch, which is the one place the user turns routing off
// from this page.
//
// Turning it *off* is a single call — the backend restores the agent's own
// configuration — and the row has to follow, because that list is where the
// answer to "did it work" lives. Turning one on deliberately does not happen
// here (it walks the Apps onboarding: enable → route → rewrite), so this file
// asserts both directions as they are rather than as symmetry would have them.
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { en } from "@/i18n/en";
import type { AppSettings } from "@/api/types";

import Settings from "./Settings";

const { apiMock } = vi.hoisted(() => ({
  apiMock: {
    getSettings: vi.fn(),
    getCurrencyMeta: vi.fn(),
    getFooterStats: vi.fn(),
    getPendingUpdate: vi.fn(),
    setTakeover: vi.fn(),
    updateSettings: vi.fn(),
    openLogFolder: vi.fn(),
    checkAppUpdate: vi.fn(),
    downloadAndInstallAppUpdate: vi.fn(),
  },
}));

vi.mock("../api/client", () => ({ api: apiMock }));

/** A whole settings blob, as the backend hands one over — the takeover list is
    what this screen's list renders, and the rest keeps the page whole. */
function settingsWith(codexTakenOver: boolean): AppSettings {
  return {
    language: "en",
    theme: "dark",
    autostart: true,
    close_to_tray: true,
    gateway_listen: "127.0.0.1:8317",
    takeovers: [
      {
        agent: "codex",
        label: "Codex",
        placeholder_key: codexTakenOver ? "kw-ag-codex-test" : null,
        enabled: codexTakenOver,
        additive: false,
      },
    ],
    custom_agents: [],
    auto_failover: true,
    request_logs: true,
    log_retention_days: 30,
    stream_first_byte_secs: 120,
    stream_idle_secs: 120,
    cost_alert: true,
    preferred_currency: "CNY",
    auto_check_update: true,
    dismissed_update: null,
    tz_offset_minutes: 480,
    hub_logged_in: false,
    hub_url: "https://hub.example",
    shelf_sort: null,
    shelf_view: null,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  apiMock.getCurrencyMeta.mockResolvedValue({
    preferred: "CNY",
    currencies: ["CNY"],
    exchange_rates: {},
  });
  apiMock.getFooterStats.mockResolvedValue({
    version: "v0.1.9",
    today_requests: 0,
    today_tokens: 0,
    hub_synced: false,
  });
  apiMock.getPendingUpdate.mockResolvedValue(null);
  apiMock.setTakeover.mockResolvedValue(undefined);
});

describe("the takeover switch", () => {
  it("re-reads the settings after turning one off, so the row follows", async () => {
    apiMock.getSettings
      .mockResolvedValueOnce(settingsWith(true)) // mount: codex is taken over
      .mockResolvedValueOnce(settingsWith(false)); // after the call: it is not
    const user = userEvent.setup();
    render(<Settings />);

    const sw = await screen.findByRole("switch", { name: "Codex" });
    expect(sw).toBeChecked();
    expect(screen.getByText(en.settings.takenOver)).toBeInTheDocument();

    await user.click(sw);

    await waitFor(() => expect(apiMock.setTakeover).toHaveBeenCalledWith("codex", false));
    // Re-read, not locally patched: the state the user sees is the one the
    // backend reports, and the row now offers the Apps route back in.
    await waitFor(() => expect(apiMock.getSettings).toHaveBeenCalledTimes(2));
    expect(await screen.findByText(en.settings.notTakenOver)).toBeInTheDocument();
    expect(screen.queryByRole("switch", { name: "Codex" })).toBeNull();
    expect(screen.getByRole("button", { name: en.settings.enableInApps })).toBeInTheDocument();
  });

  it("reports a refused takeover instead of leaving the switch on in silence", async () => {
    apiMock.getSettings.mockResolvedValue(settingsWith(true));
    apiMock.setTakeover.mockRejectedValue(new Error("restore failed"));
    const user = userEvent.setup();
    render(<Settings />);

    await user.click(await screen.findByRole("switch", { name: "Codex" }));

    expect(await screen.findByText(/codex: Error: restore failed/)).toBeInTheDocument();
    // The switch is still on, because it still is: nothing was restored.
    expect(screen.getByRole("switch", { name: "Codex" })).toBeChecked();
  });
});

// The two waits a stream can die in. The gateway enforces them; this screen is
// where they are set, so what has to hold here is that the stored value is what
// is on screen — a row that reads as "off" while the gateway is holding a
// two-minute limit is worse than no row at all.
describe("the streaming timeouts", () => {
  it("shows the stored waits", async () => {
    apiMock.getSettings.mockResolvedValue(settingsWith(true));
    render(<Settings />);

    expect(await screen.findByText(en.settings.streamFirstByte)).toBeInTheDocument();
    expect(screen.getByText(en.settings.streamIdle)).toBeInTheDocument();
    // Both fixtures carry 120s, one on each side of the stream.
    expect(
      screen.getAllByText(en.settings.seconds.replace("{count}", "120")),
    ).toHaveLength(2);
  });

  it("says off when a limit is off, rather than showing a zero-second wait", async () => {
    apiMock.getSettings.mockResolvedValue({ ...settingsWith(true), stream_idle_secs: 0 });
    render(<Settings />);

    expect(await screen.findByText(en.settings.streamIdle)).toBeInTheDocument();
    expect(screen.getByText(en.settings.off)).toBeInTheDocument();
  });

  it("patches the wait the row owns", async () => {
    apiMock.getSettings.mockResolvedValue(settingsWith(true));
    apiMock.updateSettings.mockResolvedValue({
      ...settingsWith(true),
      stream_idle_secs: 0,
    });
    const user = userEvent.setup();
    render(<Settings />);

    // Named, so the control is findable — by a reader and by this test.
    await user.click(await screen.findByRole("combobox", { name: en.settings.streamIdle }));
    await user.click(await screen.findByRole("option", { name: en.settings.off }));

    // Its own key, not the neighbouring row's: the two rows share a control type
    // and differ only in which field they carry.
    await waitFor(() =>
      expect(apiMock.updateSettings).toHaveBeenCalledWith({ stream_idle_secs: 0 }),
    );
  });
});
