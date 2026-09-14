// The Apps screen's wiring, which the Rust tests cannot reach: they prove the
// view models are right, but "does the row disappear" is a question about this
// component. Two of them are here because both were the subject of a fix —
// a route whose agent has its own config back must not render as live, and an
// unbind has to re-read the list rather than leave the All tab claiming what
// the agent tab just dropped.
//
// The API is mocked at the `client` seam, which is the only place the screens
// touch data (`src/api/client.ts` picks the Tauri or the browser-dev
// implementation there), so nothing below knows the difference.
import { render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { en } from "@/i18n/en";
import type { AgentRoute, AppSettings, Provider } from "@/api/types";

import Providers from "./Providers";

const { apiMock } = vi.hoisted(() => ({
  apiMock: {
    getGatewayStatus: vi.fn(),
    listProviders: vi.fn(),
    getAgentRoutes: vi.fn(),
    getSettings: vi.fn(),
    getPlanQuota: vi.fn(),
    removeAgentBinding: vi.fn(),
    addAgentBinding: vi.fn(),
    reorderAgentBindings: vi.fn(),
    updateAgentBinding: vi.fn(),
    setTakeover: vi.fn(),
    applyAgentRoute: vi.fn(),
    updateAgentStrategy: vi.fn(),
    deleteProvider: vi.fn(),
  },
}));

vi.mock("../api/client", () => ({ api: apiMock }));

/** A provider row as the screen receives it. `agents`/`serving_agents`/`is_current`
    are the fields the unbind has to change, so they are the ones parameterized. */
function deepseek(over: Partial<Provider> = {}): Provider {
  return {
    id: "deepseek",
    name: "DeepSeek",
    logo_char: "D",
    logo_color: "#4D6BFE",
    endpoint: "api.deepseek.com",
    protocol: "openai",
    endpoint_note: "OpenAI-compatible",
    billing: "payg",
    enabled: true,
    agents: ["codex"],
    serving_agents: ["codex"],
    is_current: true,
    health: { state: "ok", latency_ms: 120 },
    usage: null,
    ...over,
  };
}

/** codex's stored route with the given candidates, as `getAgentRoutes` answers it. */
function codexRoute(providerIds: string[]): AgentRoute {
  return {
    agent: "codex",
    strategy: "single",
    config: null,
    bindings: providerIds.map((id, i) => ({
      provider_id: id,
      provider_name: "DeepSeek",
      logo_char: "D",
      logo_color: "#4D6BFE",
      priority: i,
      weight: 1,
      win_start: null,
      win_end: null,
      enabled: true,
    })),
  };
}

/** The settings blob `getSettings` answers with. This screen reads only the
    takeover list — whether an agent is routing through the gateway at all — so
    the fixture carries that one field rather than a whole settings page. */
function settingsWith(codexTakenOver: boolean): AppSettings {
  return {
    takeovers: [
      {
        agent: "codex",
        label: "Codex",
        placeholder_key: codexTakenOver ? "kw-ag-codex-test" : null,
        enabled: codexTakenOver,
        additive: false,
      },
    ],
  } as unknown as AppSettings;
}

/** Render the screen on codex's tab, with the API answering `routes`/`settings`. */
function renderCodexTab(over: {
  providers: Provider[];
  routes: AgentRoute[];
  codexTakenOver: boolean;
}) {
  apiMock.listProviders.mockResolvedValue(over.providers);
  apiMock.getAgentRoutes.mockResolvedValue(over.routes);
  apiMock.getSettings.mockResolvedValue(settingsWith(over.codexTakenOver));
  return render(<Providers onAdd={() => {}} onEdit={() => {}} initialAgent="codex" />);
}

beforeEach(() => {
  vi.clearAllMocks();
  apiMock.getGatewayStatus.mockResolvedValue({ running: true, port: 8317, blocked: [] });
  apiMock.removeAgentBinding.mockResolvedValue(undefined);
});

describe("an agent tab", () => {
  it("unbinds a candidate and re-reads the list", async () => {
    renderCodexTab({
      providers: [deepseek()],
      routes: [codexRoute(["deepseek"])],
      codexTakenOver: true,
    });
    const button = await screen.findByLabelText(en.providers.removeFromRouteAria);

    // What the *next* read answers — the state the mutation leaves behind: the
    // candidate is off the route, and the provider no longer names the agent.
    // Both halves matter, because both tabs render from these two calls.
    apiMock.listProviders.mockResolvedValue([
      deepseek({ agents: [], serving_agents: [], is_current: false }),
    ]);
    apiMock.getAgentRoutes.mockResolvedValue([codexRoute([])]);
    button.click();

    await waitFor(() =>
      expect(apiMock.removeAgentBinding).toHaveBeenCalledWith("codex", "deepseek"),
    );
    // Refetched rather than only mutated: the row and its button go away, and
    // the tab falls back to the "no candidates yet" panel.
    await waitFor(() =>
      expect(screen.queryByLabelText(en.providers.removeFromRouteAria)).toBeNull(),
    );
    expect(apiMock.listProviders).toHaveBeenCalledTimes(2);
    expect(
      screen.getByText(en.providers.takenOverBody),
      "the tab reports an empty route rather than keeping a stale row",
    ).toBeInTheDocument();
  });

  it("shows the takeover onboarding, not a route its agent is not serving", async () => {
    // Exactly the stale state the earlier fix was about: the store still holds
    // codex's route, while codex's own config points at its provider again.
    renderCodexTab({
      providers: [deepseek({ agents: [], serving_agents: [], is_current: false })],
      routes: [codexRoute(["deepseek"])],
      codexTakenOver: false,
    });

    expect(
      await screen.findByText(en.providers.startManaging.replace("{agent}", "Codex")),
    ).toBeInTheDocument();
    expect(screen.queryByLabelText(en.providers.removeFromRouteAria)).toBeNull();
    // Nothing of the dormant route is on screen — not the candidate row, not
    // the provider it named.
    expect(screen.queryByText("DeepSeek")).toBeNull();
  });
});
