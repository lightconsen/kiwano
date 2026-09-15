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
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { en } from "@/i18n/en";
import type {
  AgentDetect,
  AgentLimit,
  AgentRoute,
  AppSettings,
  CustomAgent,
  Provider,
} from "@/api/types";

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
    setAgentLimit: vi.fn(),
    deleteProvider: vi.fn(),
    addCustomAgent: vi.fn(),
    removeCustomAgent: vi.fn(),
    testProviderLatency: vi.fn(),
    setProviderEnabled: vi.fn(),
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
    // What the backend produces for an enabled provider now that no probe
    // writes a verdict: neutral, with nothing measured.
    health: { state: "idle", latency_ms: null },
    usage: null,
    ...over,
  };
}

/** codex's stored route with the given candidates, as `getAgentRoutes` answers it.
    `limit` rides the route because that is the fetch the tab already makes — it
    is not part of the strategy, and passes through here untouched. */
function codexRoute(providerIds: string[], limit: AgentLimit | null = null): AgentRoute {
  return {
    agent: "codex",
    strategy: "single",
    config: null,
    limit,
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

/** The settings blob `getSettings` answers with. This screen reads the takeover
    list (is an agent routing at all), the user's own agents and the listen
    address; the rest of a settings page is not this file's business. */
function settingsWith(codexTakenOver: boolean, custom: CustomAgent[] = []): AppSettings {
  return {
    gateway_listen: "127.0.0.1:8317",
    custom_agents: custom,
    takeovers: [
      {
        agent: "codex",
        label: "Codex",
        placeholder_key: codexTakenOver ? "kw-ag-codex-test" : null,
        enabled: codexTakenOver,
        additive: false,
        // Two files, as `takeover_paths` gives codex.
        config_paths: ["~/.codex/config.toml", "~/.codex/auth.json"],
      },
    ],
  } as unknown as AppSettings;
}

/** Render the screen on codex's tab, with the API answering `routes`/`settings`. */
function renderCodexTab(over: {
  providers: Provider[];
  routes: AgentRoute[];
  codexTakenOver: boolean;
  custom?: CustomAgent[];
}) {
  apiMock.listProviders.mockResolvedValue(over.providers);
  apiMock.getAgentRoutes.mockResolvedValue(over.routes);
  apiMock.getSettings.mockResolvedValue(settingsWith(over.codexTakenOver, over.custom));
  return render(<Providers onAdd={() => {}} onEdit={() => {}} initialAgent="codex" />);
}

beforeEach(() => {
  vi.clearAllMocks();
  apiMock.getGatewayStatus.mockResolvedValue({ running: true, port: 8317, blocked: [] });
  apiMock.removeAgentBinding.mockResolvedValue(undefined);
  apiMock.setAgentLimit.mockResolvedValue(undefined);
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

describe("a user-defined agent", () => {
  const longTasks: CustomAgent = {
    id: "long-tasks-3f9a",
    label: "Long tasks",
    note: "cheap by day, batch at night",
    placeholder_key: "kw-ag-long-tasks-3f9a-b7e1",
  };

  /** A route for the user-defined agent: `bindings` empty or not. */
  function customRoute(providerIds: string[]): AgentRoute {
    return { ...codexRoute([]), agent: longTasks.id, bindings: codexRoute(providerIds).bindings };
  }

  it("gets its own segment, and a tab that says how a client reaches it", async () => {
    const user = userEvent.setup();
    apiMock.listProviders.mockResolvedValue([deepseek()]);
    apiMock.getAgentRoutes.mockResolvedValue([customRoute(["deepseek"])]);
    apiMock.getSettings.mockResolvedValue(settingsWith(true, [longTasks]));
    render(<Providers onAdd={() => {}} onEdit={() => {}} />);

    // The segment strip carries it by name — nothing was installed, because
    // there is nothing to install.
    const segment = await screen.findByRole("button", { name: longTasks.label });
    await user.click(segment);

    // The tab names the agent: the strip shows a letter avatar, so a page with
    // no name on it is one you have to identify from the highlight over there.
    expect(await screen.findByText(longTasks.label)).toBeInTheDocument();
    expect(screen.getByText(longTasks.note!)).toBeInTheDocument();
    // Its candidate is a row like any agent's…
    expect(screen.getByLabelText(en.providers.removeFromRouteAria)).toBeInTheDocument();
    // …and there is nothing to enable: a route has no config to take over.
    expect(screen.queryByText(en.providers.enableKiwano)).toBeNull();
    expect(screen.queryByText(en.providers.startManagingBody.replace("{agent}", longTasks.label))).toBeNull();
  });

  it("asks for a candidate before it has any", async () => {
    apiMock.listProviders.mockResolvedValue([deepseek({ agents: [] })]);
    apiMock.getAgentRoutes.mockResolvedValue([customRoute([])]);
    apiMock.getSettings.mockResolvedValue(settingsWith(true, [longTasks]));
    render(<Providers onAdd={() => {}} onEdit={() => {}} />);

    await userEvent.setup().click(await screen.findByRole("button", { name: longTasks.label }));

    // The route-only panel: bind one, and the copy says what the candidates are
    // for — not the takeover copy, which would be about a config file that does
    // not exist here.
    expect(
      await screen.findByText(en.providers.routeEmptyTitle.replace("{agent}", longTasks.label)),
    ).toBeInTheDocument();
    expect(screen.getByText(en.providers.routeEmptyBody)).toBeInTheDocument();
    expect(screen.queryByText(en.providers.enableKiwano)).toBeNull();
  });

  it("is created from the + in the strip, and opens on its tab", async () => {
    const user = userEvent.setup();
    const created: CustomAgent = {
      id: "nightly-7c21",
      label: "Nightly",
      note: null,
      placeholder_key: "kw-ag-nightly-7c21-11aa",
    };
    apiMock.listProviders.mockResolvedValue([]);
    apiMock.getAgentRoutes.mockResolvedValue([{ ...codexRoute([]), agent: created.id }]);
    apiMock.getSettings.mockResolvedValue(settingsWith(true, []));
    apiMock.addCustomAgent.mockResolvedValue(created);
    render(<Providers onAdd={() => {}} onEdit={() => {}} />);

    await user.click(await screen.findByRole("button", { name: en.providers.newAgent }));
    // The next settings read is the one the creation triggers.
    apiMock.getSettings.mockResolvedValue(settingsWith(true, [created]));
    await user.type(screen.getByRole("textbox", { name: en.providers.agentName }), created.label);
    await user.click(screen.getByRole("button", { name: en.providers.createAgent }));

    await waitFor(() =>
      expect(apiMock.addCustomAgent).toHaveBeenCalledWith(created.label, ""),
    );
    // It opens on the new agent's tab, which names it: what the user wants next
    // is to bind a provider, and that is what that tab asks for.
    expect(await screen.findByText(created.label)).toBeInTheDocument();
    expect(
      screen.getByText(en.providers.routeEmptyTitle.replace("{agent}", created.label)),
    ).toBeInTheDocument();
  });

  it("can change its strategy — the route is the whole of it", async () => {
    const user = userEvent.setup();
    apiMock.listProviders.mockResolvedValue([deepseek()]);
    apiMock.getAgentRoutes.mockResolvedValue([customRoute(["deepseek"])]);
    apiMock.getSettings.mockResolvedValue(settingsWith(true, [longTasks]));
    apiMock.updateAgentStrategy.mockResolvedValue(undefined);
    render(<Providers onAdd={() => {}} onEdit={() => {}} />);

    await user.click(await screen.findByRole("button", { name: longTasks.label }));

    // The strategy panel is the tab's control over what the route *is*. It used
    // to be gated on a takeover, which a user-defined agent never has — so the
    // tab offered no way to change the one thing it exists for.
    const picker = await screen.findByRole("combobox", {
      name: en.strategy.ariaFor.replace("{agent}", longTasks.label),
    });
    await user.click(picker);
    await user.click(await screen.findByRole("option", { name: en.strategy.roundrobin }));

    await waitFor(() =>
      expect(apiMock.updateAgentStrategy).toHaveBeenCalledWith(
        longTasks.id,
        "roundrobin",
        null,
      ),
    );
    // …and the screen re-reads the routes, so the panel shows what was chosen.
    expect(apiMock.getAgentRoutes).toHaveBeenCalledTimes(2);
  });

  it("keeps its credentials behind the icon beside its name", async () => {
    const user = userEvent.setup();
    apiMock.listProviders.mockResolvedValue([deepseek()]);
    apiMock.getAgentRoutes.mockResolvedValue([customRoute(["deepseek"])]);
    apiMock.getSettings.mockResolvedValue(settingsWith(true, [longTasks]));
    render(<Providers onAdd={() => {}} onEdit={() => {}} />);
    await user.click(await screen.findByRole("button", { name: longTasks.label }));

    // Not on the tab: credentials are read once and pasted, not watched.
    const opens = en.providers.accessFor.replace("{agent}", longTasks.label);
    expect(screen.queryByText(longTasks.placeholder_key!)).toBeNull();

    await user.click(screen.getByRole("button", { name: opens }));

    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText(opens)).toBeInTheDocument();
    expect(within(dialog).getByText("http://127.0.0.1:8317")).toBeInTheDocument();
    expect(within(dialog).getByText(longTasks.placeholder_key!)).toBeInTheDocument();
    // One copy button per value, named for what it does and carrying no text of
    // its own — an icon keeps the value the widest thing on the line.
    const copies = within(dialog).getAllByRole("button", { name: en.common.copy });
    expect(copies.map((b) => b.textContent)).toEqual(["", ""]);
  });

  it("is deleted in two clicks, and its segment goes with it", async () => {
    const user = userEvent.setup();
    apiMock.listProviders.mockResolvedValue([]);
    apiMock.getAgentRoutes.mockResolvedValue([customRoute([])]);
    apiMock.getSettings.mockResolvedValue(settingsWith(true, [longTasks]));
    apiMock.removeCustomAgent.mockResolvedValue(undefined);
    render(<Providers onAdd={() => {}} onEdit={() => {}} />);

    await user.click(await screen.findByRole("button", { name: longTasks.label }));

    // Deleting lives in the agent's own dialog, not on the tab: the row you
    // click around in keeps no destructive control.
    expect(screen.queryByRole("button", { name: en.providers.deleteAgent })).toBeNull();
    await user.click(
      screen.getByRole("button", {
        name: en.providers.accessFor.replace("{agent}", longTasks.label),
      }),
    );
    const del = await screen.findByRole("button", { name: en.providers.deleteAgent });
    await user.click(del);
    // The first click only arms it…
    expect(apiMock.removeCustomAgent).not.toHaveBeenCalled();
    apiMock.getSettings.mockResolvedValue(settingsWith(true, []));
    await user.click(del);

    await waitFor(() => expect(apiMock.removeCustomAgent).toHaveBeenCalledWith(longTasks.id));
    // …and the tab is gone from the strip, with the screen back on All.
    await waitFor(() =>
      expect(screen.queryByRole("button", { name: longTasks.label })).toBeNull(),
    );
  });
});

describe("an id nothing knows", () => {
  it("still renders — a deleted agent's rows are still in the lists", async () => {
    // The regression the resolver exists for: a provider bound to an agent that
    // is no longer defined used to hit a non-null assertion and take the screen
    // down with it.
    apiMock.listProviders.mockResolvedValue([
      deepseek({ agents: ["ghost-77ab"], serving_agents: [], is_current: false }),
    ]);
    apiMock.getAgentRoutes.mockResolvedValue([]);
    apiMock.getSettings.mockResolvedValue(settingsWith(true, []));
    render(<Providers onAdd={() => {}} onEdit={() => {}} />);

    expect(await screen.findByText("DeepSeek")).toBeInTheDocument();
    // Its avatar is derived from the id: the first letter it has.
    expect(screen.getByText("G")).toBeInTheDocument();
  });
});

describe("the latency test", () => {
  /** The All tab, with one provider and no routes. */
  async function renderAll() {
    apiMock.listProviders.mockResolvedValue([deepseek()]);
    apiMock.getAgentRoutes.mockResolvedValue([]);
    apiMock.getSettings.mockResolvedValue(settingsWith(true, []));
    render(<Providers onAdd={() => {}} onEdit={() => {}} />);
    return userEvent.setup();
  }

  it("times one prompt and shows the number on the row", async () => {
    const user = await renderAll();
    apiMock.testProviderLatency.mockResolvedValue({
      provider_id: "deepseek",
      model: "deepseek-chat",
      latency_ms: 1240,
      status: 200,
      error: null,
    });

    await user.click(await screen.findByRole("button", { name: en.providers.testLatency }));

    await waitFor(() => expect(apiMock.testProviderLatency).toHaveBeenCalledWith("deepseek"));
    // The number lands on the button, not in the Status cell: the cell is for
    // standing facts, and this is a measurement the click just asked for.
    expect(await screen.findByText("1240ms")).toBeInTheDocument();
  });

  it("reports a refused prompt as a failure, not as a fast provider", async () => {
    const user = await renderAll();
    apiMock.testProviderLatency.mockResolvedValue({
      provider_id: "deepseek",
      model: "deepseek-chat",
      latency_ms: 180,
      status: 401,
      error: "invalid API key",
    });

    const button = await screen.findByRole("button", { name: en.providers.testLatency });
    await user.click(button);

    // "180ms" on a 401 would read as a very fast provider; the reason is what
    // the row has to say, and it says it in the tooltip.
    expect(await screen.findByText(en.providers.testLatencyFailed)).toBeInTheDocument();
    await waitFor(() =>
      expect(screen.getByRole("button", { name: en.providers.testLatency })).toHaveAttribute(
        "title",
        "invalid API key",
      ),
    );
  });
});

describe("deleting a provider that is in a route", () => {
  /** codex's route, with the candidates named as the row would show them. */
  function routeOf(...providers: [string, string][]): AgentRoute {
    return {
      agent: "codex",
      strategy: "single",
      config: null,
      limit: null,
      bindings: providers.map(([id, name], i) => ({
        provider_id: id,
        provider_name: name,
        logo_char: name[0],
        logo_color: "#4D6BFE",
        priority: i,
        weight: 1,
        win_start: null,
        win_end: null,
        enabled: true,
      })),
    };
  }

  async function arm(route: AgentRoute) {
    const user = userEvent.setup();
    apiMock.listProviders.mockResolvedValue([deepseek()]);
    apiMock.getAgentRoutes.mockResolvedValue([route]);
    apiMock.getSettings.mockResolvedValue(settingsWith(true, []));
    render(<Providers onAdd={() => {}} onEdit={() => {}} />);
    await user.click(await screen.findByRole("button", { name: en.common.delete }));
    return user;
  }

  it("names the agents and the candidate that takes over", async () => {
    await arm(routeOf(["deepseek", "DeepSeek"], ["kimi", "Kimi (Moonshot)"]));

    // The consequence is on the row itself, at the moment of the decision —
    // not in a tooltip the reader has to go looking for.
    const hint = await screen.findByText(/removed from Codex/);
    expect(hint).toHaveTextContent(
      en.providers.delPromotes
        .replace("{provider}", "Kimi (Moonshot)")
        .replace("{agent}", "Codex"),
    );
  });

  it("says when a route is left with nothing", async () => {
    await arm(routeOf(["deepseek", "DeepSeek"]));

    const hint = await screen.findByText(/removed from Codex/);
    // The one case where deleting silently breaks traffic: the agent has no
    // candidate behind it.
    expect(hint).toHaveTextContent(en.providers.delEmpties.replace("{agent}", "Codex"));
    expect(hint).not.toHaveTextContent("becomes the primary");
  });

  it("says nothing about a route that does not mention it", async () => {
    // A promotion is also what roundrobin does *not* have: every candidate takes
    // turns there, so the order is a priority, not a primary.
    await arm({ ...routeOf(["deepseek", "DeepSeek"], ["kimi", "Kimi (Moonshot)"]), strategy: "roundrobin" });

    const hint = await screen.findByText(/removed from Codex/);
    expect(hint).not.toHaveTextContent("becomes the primary");
  });
});

describe("parking a provider", () => {
  it("stops it without deleting it, and re-reads the list", async () => {
    const user = userEvent.setup();
    apiMock.listProviders.mockResolvedValue([deepseek()]);
    apiMock.getAgentRoutes.mockResolvedValue([]);
    apiMock.getSettings.mockResolvedValue(settingsWith(true, []));
    apiMock.setProviderEnabled.mockResolvedValue(undefined);
    render(<Providers onAdd={() => {}} onEdit={() => {}} />);

    // The glyph is the action: a working provider offers a stop.
    const stop = await screen.findByRole("button", { name: en.providers.disable });
    expect(stop.querySelector("svg.lucide-pause")).not.toBeNull();

    await user.click(stop);

    await waitFor(() =>
      expect(apiMock.setProviderEnabled).toHaveBeenCalledWith("deepseek", false),
    );
    // The row stays and the screen re-reads, so what it shows is the store's
    // answer rather than a locally flipped flag.
    expect(apiMock.listProviders).toHaveBeenCalledTimes(2);
    expect(screen.getByText("DeepSeek")).toBeInTheDocument();
  });

  it("offers to start a parked one", async () => {
    apiMock.listProviders.mockResolvedValue([deepseek({ enabled: false })]);
    apiMock.getAgentRoutes.mockResolvedValue([]);
    apiMock.getSettings.mockResolvedValue(settingsWith(true, []));
    render(<Providers onAdd={() => {}} onEdit={() => {}} />);

    // …and a parked one offers a start (▶, the enable glyph), tooltip included:
    // the pair says which way the row points, the title says what the click does.
    const start = await screen.findByRole("button", { name: en.providers.enable });
    expect(start.querySelector("svg.lucide-play")).not.toBeNull();
    expect(start.querySelector("svg.lucide-pause")).toBeNull();
    expect(start).toHaveAttribute("title", en.providers.enableTitle);
  });
});

describe("the refresh button", () => {
  /** Only pi was installed when the app launched: the strip has no Codex tab. */
  const piOnly: AgentDetect[] = [
    { agent: "pi", installed: true, path: "/usr/bin/pi" },
    { agent: "codex", installed: false, path: null },
  ];

  it("re-probes the machine, so a CLI installed since launch gets a tab", async () => {
    const user = userEvent.setup();
    const onRedetect = vi.fn();
    apiMock.listProviders.mockResolvedValue([deepseek()]);
    apiMock.getAgentRoutes.mockResolvedValue([]);
    apiMock.getSettings.mockResolvedValue(settingsWith(true, []));
    const view = render(
      <Providers onAdd={() => {}} onEdit={() => {}} agentDetect={piOnly} onRedetect={onRedetect} />,
    );

    // The detection result gates the strip: codex is not installed, so no tab.
    expect(await screen.findByRole("button", { name: "Pi" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Codex" })).toBeNull();

    await user.click(await screen.findByRole("button", { name: en.common.refresh }));

    await waitFor(() => expect(onRedetect).toHaveBeenCalledTimes(1));
    // The button's other two jobs are unchanged: the local reads happen too.
    expect(apiMock.listProviders).toHaveBeenCalledTimes(2);

    // What the probe answers arrives back as a prop (App owns the result): codex
    // is installed now, and its tab is in the strip without a restart.
    view.rerender(
      <Providers
        onAdd={() => {}}
        onEdit={() => {}}
        agentDetect={[
          { agent: "pi", installed: true, path: "/usr/bin/pi" },
          { agent: "codex", installed: true, path: "/usr/local/bin/codex" },
        ]}
        onRedetect={onRedetect}
      />,
    );
    expect(await screen.findByRole("button", { name: "Codex" })).toBeInTheDocument();
  });

  it("does not re-probe on an ordinary refetch — only a click pays for it", async () => {
    const user = userEvent.setup();
    const onRedetect = vi.fn();
    apiMock.listProviders.mockResolvedValue([deepseek()]);
    apiMock.getAgentRoutes.mockResolvedValue([]);
    apiMock.getSettings.mockResolvedValue(settingsWith(true, []));
    apiMock.setProviderEnabled.mockResolvedValue(undefined);
    render(<Providers onAdd={() => {}} onEdit={() => {}} onRedetect={onRedetect} />);

    // Parking a provider re-reads the local data; that refetch is the generic
    // "something changed" callback, and must not drag a login shell with it.
    await user.click(await screen.findByRole("button", { name: en.providers.disable }));

    await waitFor(() => expect(apiMock.listProviders).toHaveBeenCalledTimes(2));
    expect(onRedetect).not.toHaveBeenCalled();
  });
});

// The agent's own ceiling, which lives in the agent's settings dialog — not in
// the strategy panel, because it holds under every strategy rather than being
// part of any one of them.
describe("the agent's own limit", () => {
  /** Open the agent's settings from the row above its table. */
  async function openSettings(user: ReturnType<typeof userEvent.setup>) {
    await user.click(
      await screen.findByLabelText(
        en.providers.agentSettingsFor.replace("{agent}", "Codex"),
      ),
    );
  }

  it("reads out the stored ceiling", async () => {
    const user = userEvent.setup();
    renderCodexTab({
      providers: [deepseek()],
      routes: [
        codexRoute(["deepseek"], {
          period_limit: 50,
          limit_unit: "CNY",
          reset_period: "monthly",
        }),
      ],
      codexTakenOver: true,
    });
    await openSettings(user);

    expect(screen.getByText(en.strategy.limitLabel)).toBeInTheDocument();
    expect(screen.getByText("50 CNY per month")).toBeInTheDocument();
  });

  it("says no limit rather than showing a blank", async () => {
    const user = userEvent.setup();
    renderCodexTab({
      providers: [deepseek()],
      routes: [codexRoute(["deepseek"])],
      codexTakenOver: true,
    });
    await openSettings(user);

    expect(screen.getByText(en.strategy.limitNone)).toBeInTheDocument();
  });

  it("saves the value the section holds", async () => {
    const user = userEvent.setup();
    renderCodexTab({
      providers: [deepseek()],
      routes: [codexRoute(["deepseek"])],
      codexTakenOver: true,
    });
    await openSettings(user);

    await user.type(await screen.findByLabelText(en.strategy.limitAmountAria), "25");
    await user.click(screen.getByRole("button", { name: en.common.save }));

    // The defaults the section opens with: counted in requests, per day.
    await waitFor(() =>
      expect(apiMock.setAgentLimit).toHaveBeenCalledWith("codex", {
        period_limit: 25,
        limit_unit: null,
        reset_period: "day",
      }),
    );
  });

  it("is on a user-defined agent's dialog too, where it is the only way in", async () => {
    const user = userEvent.setup();
    const longTasks: CustomAgent = {
      id: "long-tasks-3f9a",
      label: "Long tasks",
      note: null,
      placeholder_key: "kw-ag-long-tasks-3f9a-b7e1",
    };
    apiMock.listProviders.mockResolvedValue([deepseek()]);
    apiMock.getAgentRoutes.mockResolvedValue([
      { ...codexRoute(["deepseek"]), agent: longTasks.id },
    ]);
    apiMock.getSettings.mockResolvedValue(settingsWith(true, [longTasks]));
    render(<Providers onAdd={() => {}} onEdit={() => {}} />);

    await user.click(await screen.findByRole("button", { name: longTasks.label }));
    await user.click(
      screen.getByLabelText(en.providers.accessFor.replace("{agent}", longTasks.label)),
    );

    await user.type(await screen.findByLabelText(en.strategy.limitAmountAria), "5");
    await user.click(screen.getByRole("button", { name: en.common.save }));

    await waitFor(() =>
      expect(apiMock.setAgentLimit).toHaveBeenCalledWith(longTasks.id, {
        period_limit: 5,
        limit_unit: null,
        reset_period: "day",
      }),
    );
  });
});

// A built-in agent's tab names itself, and names the file behind it. The strip
// above is icons only, so without this row the page has no agent name on it at
// all — and no way to find the config a takeover rewrites, which is what someone
// reaches for by hand when the takeover is the problem.
describe("a built-in agent's own row", () => {
  it("names the agent and the files a takeover would rewrite", async () => {
    renderCodexTab({
      providers: [deepseek()],
      routes: [codexRoute(["deepseek"])],
      codexTakenOver: true,
    });

    // Codex keeps two files; the row shows the first and counts the rest, and
    // the dialog below lists them all.
    expect(await screen.findByText("~/.codex/config.toml +1")).toBeInTheDocument();
    expect(
      screen.getByLabelText(en.providers.agentSettingsFor.replace("{agent}", "Codex")),
    ).toBeInTheDocument();
  });

  it("still names it before the agent is taken over, and offers to enable it", async () => {
    const user = userEvent.setup();
    renderCodexTab({
      providers: [deepseek()],
      routes: [codexRoute(["deepseek"])],
      codexTakenOver: false,
    });

    // The config file exists whether or not Kiwano has rewritten it, and this is
    // the case where someone wants to see it most.
    await user.click(
      await screen.findByLabelText(en.providers.agentSettingsFor.replace("{agent}", "Codex")),
    );

    expect(screen.getByText(en.providers.agentNotRouted)).toBeInTheDocument();
    expect(screen.getByText("~/.codex/config.toml")).toBeInTheDocument();
    expect(screen.getByText("~/.codex/auth.json")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: en.providers.enableKiwano }));
    await waitFor(() => expect(apiMock.setTakeover).toHaveBeenCalledWith("codex", true));
  });

  it("restores the original config from the dialog", async () => {
    const user = userEvent.setup();
    renderCodexTab({
      providers: [deepseek()],
      routes: [codexRoute(["deepseek"])],
      codexTakenOver: true,
    });

    await user.click(
      await screen.findByLabelText(en.providers.agentSettingsFor.replace("{agent}", "Codex")),
    );
    expect(screen.getByText(en.providers.agentRouted)).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: en.providers.restoreOriginal }));
    await waitFor(() => expect(apiMock.setTakeover).toHaveBeenCalledWith("codex", false));
  });

  it("is not on the all-agents tab, and not on a user-defined agent's", async () => {
    const user = userEvent.setup();
    const longTasks: CustomAgent = {
      id: "long-tasks-3f9a",
      label: "Long tasks",
      note: null,
      placeholder_key: "kw-ag-long-tasks-3f9a-b7e1",
    };
    apiMock.listProviders.mockResolvedValue([deepseek()]);
    apiMock.getAgentRoutes.mockResolvedValue([
      { ...codexRoute([]), agent: longTasks.id, bindings: codexRoute(["deepseek"]).bindings },
    ]);
    apiMock.getSettings.mockResolvedValue(settingsWith(true, [longTasks]));
    render(<Providers onAdd={() => {}} onEdit={() => {}} />);

    // The All tab has no agent, so it has no agent's file to name.
    expect(
      screen.queryByLabelText(en.providers.agentSettingsFor.replace("{agent}", "Codex")),
    ).toBeNull();

    // A user-defined agent has no config file behind it, so its own tab keeps the
    // row it always had — the gear that opens its credentials — and gains none.
    await user.click(await screen.findByRole("button", { name: longTasks.label }));
    expect(
      screen.queryByLabelText(
        en.providers.agentSettingsFor.replace("{agent}", longTasks.label),
      ),
    ).toBeNull();
    expect(
      screen.getByLabelText(en.providers.accessFor.replace("{agent}", longTasks.label)),
    ).toBeInTheDocument();
  });
});
