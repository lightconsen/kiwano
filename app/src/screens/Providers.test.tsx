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
  UsageSummary,
} from "@/api/types";

import Providers from "./Providers";

const { apiMock } = vi.hoisted(() => ({
  apiMock: {
    getGatewayStatus: vi.fn(),
    listProviders: vi.fn(),
    getAgentRoutes: vi.fn(),
    getSettings: vi.fn(),
    getCurrencyMeta: vi.fn(),
    getPlanQuota: vi.fn(),
    removeAgentBinding: vi.fn(),
    addAgentBinding: vi.fn(),
    reorderAgentBindings: vi.fn(),
    updateAgentBinding: vi.fn(),
    setTakeover: vi.fn(),
    applyAgentRoute: vi.fn(),
    updateAgentStrategy: vi.fn(),
    setAgentLimits: vi.fn(),
    deleteProvider: vi.fn(),
    addCustomAgent: vi.fn(),
    updateCustomAgent: vi.fn(),
    removeCustomAgent: vi.fn(),
    agentSearchDirs: vi.fn(),
    verifyAgentDir: vi.fn(),
    setAgentDir: vi.fn(),
    clearAgentDir: vi.fn(),
    testProviderLatency: vi.fn(),
    setProviderEnabled: vi.fn(),
  },
}));

vi.mock("../api/client", () => ({ api: apiMock }));

// Browse is the only path that runs the pick-time check, so the picker has to
// be driven: the plugin is a no-op under jsdom (no Tauri IPC to call).
const { openMock } = vi.hoisted(() => ({ openMock: vi.fn() }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: openMock }));

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
    // What its figures are denominated in — the unit a ceiling on its agent can
    // be written in.
    currency: "USD",
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
function codexRoute(providerIds: string[], limits: AgentLimit[] = []): AgentRoute {
  return {
    agent: "codex",
    strategy: "single",
    config: null,
    limits,
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
        // What `AGENT_PROTOCOLS` holds for codex: `wire_api = "responses"`.
        protocols: ["openai"],
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
  // The strip only shows agents the probe found installed (or ones already
  // routed) — the codex tab needs codex in the detection result to exist.
  return render(
    <Providers
      onAdd={() => {}}
      onEdit={() => {}}
      initialAgent="codex"
      agentDetect={codexInstalled}
    />,
  );
}

/** Detection for the codex-tab tests: codex installed, the rest not. */
const codexInstalled = [{ agent: "codex", installed: true, path: "/bin/codex" }];

beforeEach(() => {
  vi.clearAllMocks();
  apiMock.getGatewayStatus.mockResolvedValue({ running: true, port: 8317, blocked: [] });
  apiMock.removeAgentBinding.mockResolvedValue(undefined);
  apiMock.setAgentLimits.mockResolvedValue(undefined);
  apiMock.getCurrencyMeta.mockResolvedValue({
    preferred: "USD",
    currencies: ["CNY", "USD"],
    exchange_rates: {},
  });
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

describe("an agent tab's row actions", () => {
  it("leaves no hole between the two buttons a primary row shows", async () => {
    // The row's Pin is hidden on the primary (it is already primary) but keeps
    // its slot so the rows of one route stay aligned. Where that slot sits is
    // the whole question: between two visible buttons it is a gap a button
    // wide, which is what this row showed.
    renderCodexTab({
      providers: [deepseek()],
      routes: [codexRoute(["deepseek"])],
      codexTakenOver: true,
    });

    const remove = await screen.findByLabelText(en.providers.removeFromRouteAria);
    expect(remove.previousElementSibling?.getAttribute("aria-label")).toBe(
      en.providers.testLatency,
    );
    // And the slot is still reserved, at the front: the pin renders on a
    // candidate that is not primary, which is what keeps the columns aligned.
    const pin = screen.getByLabelText(en.providers.makePrimary);
    expect(pin.className).toContain("invisible");
    expect(pin.nextElementSibling?.getAttribute("aria-label")).toBe(en.providers.testLatency);
  });
});

describe("a user-defined agent", () => {
  const longTasks: CustomAgent = {
    id: "long-tasks-3f9a",
    label: "Long tasks",
    note: "cheap by day, batch at night",
    protocol: "openai",
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
      protocol: "gemini",
      placeholder_key: "kw-ag-nightly-7c21-11aa",
    };
    apiMock.listProviders.mockResolvedValue([]);
    apiMock.getAgentRoutes.mockResolvedValue([{ ...codexRoute([]), agent: created.id }]);
    apiMock.getSettings.mockResolvedValue(settingsWith(true, []));
    apiMock.addCustomAgent.mockResolvedValue(created);
    render(<Providers onAdd={() => {}} onEdit={() => {}} />);

    // The + opens a menu now; creating a custom agent is its first item.
    await user.click(await screen.findByRole("button", { name: en.providers.addAgentMenu }));
    await user.click(await screen.findByRole("menuitem", { name: en.providers.newAgent }));
    // The next settings read is the one the creation triggers.
    apiMock.getSettings.mockResolvedValue(settingsWith(true, [created]));
    await user.type(screen.getByRole("textbox", { name: en.providers.agentName }), created.label);
    // The protocol opens on the common answer and is answered here, so what the
    // backend stores is what the user picked rather than the default.
    await user.click(screen.getByRole("combobox", { name: en.providers.agentProtocol }));
    await user.click(await screen.findByRole("option", { name: en.addProvider.protoGemini }));
    await user.click(screen.getByRole("button", { name: en.providers.createAgent }));

    await waitFor(() =>
      expect(apiMock.addCustomAgent).toHaveBeenCalledWith(created.label, "", "gemini"),
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
    const opens = en.providers.agentSettingsFor.replace("{agent}", longTasks.label);
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
        name: en.providers.agentSettingsFor.replace("{agent}", longTasks.label),
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

  it("says what it speaks, and lets that be changed", async () => {
    const user = userEvent.setup();
    apiMock.listProviders.mockResolvedValue([deepseek()]);
    apiMock.getAgentRoutes.mockResolvedValue([customRoute(["deepseek"])]);
    apiMock.getSettings.mockResolvedValue(settingsWith(true, [longTasks]));
    apiMock.updateCustomAgent.mockResolvedValue(longTasks);
    render(<Providers onAdd={() => {}} onEdit={() => {}} />);

    await user.click(await screen.findByRole("button", { name: longTasks.label }));
    await user.click(
      screen.getByLabelText(en.providers.agentSettingsFor.replace("{agent}", longTasks.label)),
    );
    const dialog = screen.getByRole("dialog");
    // The stored answer is the one on screen, under its own label…
    expect(within(dialog).getByText(en.providers.agentProtocol)).toBeInTheDocument();
    const picker = within(dialog).getByRole("combobox", { name: en.providers.agentProtocol });
    expect(picker).toHaveTextContent(en.addProvider.protoOpenai);

    // …and picking another saves it. The name and the note travel back
    // unchanged: the update replaces all three of the agent's own fields.
    await user.click(picker);
    await user.click(await screen.findByRole("option", { name: en.addProvider.protoAnthropic }));
    await waitFor(() =>
      expect(apiMock.updateCustomAgent).toHaveBeenCalledWith(
        longTasks.id,
        longTasks.label,
        longTasks.note,
        "anthropic",
      ),
    );
  });

  it("reads an answer nobody gave as not specified", async () => {
    const user = userEvent.setup();
    // What migration v24 leaves behind: a row that predates the field, so the
    // question is open rather than answered with a default nobody chose.
    const scratch: CustomAgent = {
      id: "scratch-91cd",
      label: "Scratch",
      note: null,
      protocol: null,
      placeholder_key: "kw-ag-scratch-91cd-40aa",
    };
    apiMock.listProviders.mockResolvedValue([deepseek()]);
    apiMock.getAgentRoutes.mockResolvedValue([
      { ...codexRoute(["deepseek"]), agent: scratch.id },
    ]);
    apiMock.getSettings.mockResolvedValue(settingsWith(true, [scratch]));
    apiMock.updateCustomAgent.mockResolvedValue(scratch);
    render(<Providers onAdd={() => {}} onEdit={() => {}} />);

    await user.click(await screen.findByRole("button", { name: scratch.label }));
    await user.click(
      screen.getByLabelText(en.providers.agentSettingsFor.replace("{agent}", scratch.label)),
    );
    const dialog = screen.getByRole("dialog");
    expect(
      within(dialog).getByRole("combobox", { name: en.providers.agentProtocol }),
    ).toHaveTextContent(en.providers.agentProtocolUnset);
  });
});

describe("a built-in the walk could not find", () => {
  /** Gemini lives somewhere the walk has no reason to look, and WorkBuddy is a
      GUI app: one of the two can be pointed at, and the menu has to know the
      difference. */
  const missing: AgentDetect[] = [
    { agent: "gemini", installed: false, path: null },
    { agent: "workbuddy", installed: false, path: null },
  ];

  /** Open the strip's + menu with `agentDetect` in hand, the API answering. */
  async function openMenu(agentDetect: AgentDetect[], onRedetect = vi.fn()) {
    apiMock.listProviders.mockResolvedValue([deepseek()]);
    apiMock.getAgentRoutes.mockResolvedValue([]);
    apiMock.getSettings.mockResolvedValue(settingsWith(true, []));
    apiMock.agentSearchDirs.mockResolvedValue(["~/.local/bin", "/opt/homebrew/bin"]);
    const user = userEvent.setup();
    const view = render(
      <Providers onAdd={() => {}} onEdit={() => {}} agentDetect={agentDetect} onRedetect={onRedetect} />,
    );
    await user.click(await screen.findByRole("button", { name: en.providers.addAgentMenu }));
    return { user, onRedetect, view };
  }

  it("is offered by name, and a directory for it is stored", async () => {
    const { user, onRedetect } = await openMenu(missing);
    const gemini = await screen.findByRole("menuitem", { name: "Gemini CLI" });
    // Only the agents with a command to point at: a GUI app cannot be helped
    // by a directory, so offering it would be a dead end.
    expect(screen.queryByRole("menuitem", { name: /WorkBuddy/ })).toBeNull();

    await user.click(gemini);

    // The dialog says what the check can and cannot establish — the whole
    // reason it is worded that way is that identity is not checkable.
    expect(
      await screen.findByText(en.providers.addAgentTitle.replace("{agent}", "Gemini CLI")),
    ).toBeInTheDocument();
    expect(screen.getByText(en.providers.addAgentNote)).toBeInTheDocument();

    // And it shows where the detector looked, so "we could not find it" is
    // something the user can check rather than take on faith — the stale
    // shell PATH this whole feature exists for is visible in that list.
    expect(apiMock.agentSearchDirs).toHaveBeenCalledWith("gemini");
    expect(
      await screen.findByText(en.providers.searchedDirs.replace("{count}", "2")),
    ).toBeInTheDocument();
    expect(screen.getByText("~/.local/bin")).toBeInTheDocument();
    expect(screen.getByText("/opt/homebrew/bin")).toBeInTheDocument();

    apiMock.setAgentDir.mockResolvedValue("1.2.3");
    await user.type(
      screen.getByRole("textbox", { name: en.providers.installDir }),
      "/opt/gemini/bin",
    );
    await user.click(screen.getByRole("button", { name: en.providers.addAgent }));

    await waitFor(() =>
      expect(apiMock.setAgentDir).toHaveBeenCalledWith("gemini", "/opt/gemini/bin"),
    );
    // What the user is waiting for is the tab, which only a re-probe produces.
    await waitFor(() => expect(onRedetect).toHaveBeenCalledTimes(1));
    expect(screen.queryByText(en.providers.addAgentNote)).toBeNull();
  });

  it("keeps the dialog open when the directory does not check out", async () => {
    const { user, onRedetect } = await openMenu(missing);
    await user.click(await screen.findByRole("menuitem", { name: "Gemini CLI" }));
    apiMock.setAgentDir.mockRejectedValue(new Error("no `gemini` in /nowhere"));

    await user.type(screen.getByRole("textbox", { name: en.providers.installDir }), "/nowhere");
    await user.click(screen.getByRole("button", { name: en.providers.addAgent }));

    // The reason, in the dialog, where the field it is about still is.
    expect(await screen.findByText("no `gemini` in /nowhere")).toBeInTheDocument();
    expect(screen.getByRole("textbox", { name: en.providers.installDir })).toBeInTheDocument();
    expect(onRedetect).not.toHaveBeenCalled();
  });

  it("shows a declared one as added, and can drop the declaration", async () => {
    const declared: AgentDetect[] = [
      { agent: "gemini", installed: true, path: "/opt/gemini/bin/gemini", manual: true },
    ];
    const { user, onRedetect } = await openMenu(declared);

    // It has a tab — a declaration is what the detector was missing — and the
    // menu still lists it, because the path can be wrong or the tool can move.
    expect(await screen.findByRole("button", { name: "Gemini CLI" })).toBeInTheDocument();
    const item = await screen.findByRole("menuitem", { name: /Gemini CLI/ });
    expect(within(item).getByText(en.providers.alreadyDeclared)).toBeInTheDocument();

    await user.click(item);
    // The dialog opens on the directory it was declared with — the binary is
    // what comes back, its parent is what the user named.
    expect(await screen.findByRole("textbox", { name: en.providers.installDir })).toHaveValue(
      "/opt/gemini/bin",
    );
    // And it does not repeat the sentence for an agent Kiwano *cannot* find,
    // which would be false here: this one it found, because it was told where.
    expect(screen.getByText(en.providers.declaredAgentNote)).toBeInTheDocument();
    expect(screen.queryByText(en.providers.addAgentNote)).toBeNull();
    // Nor a list of places it looked and did not find it — it did find it.
    expect(
      screen.queryByText(en.providers.searchedDirs.replace("{count}", "2")),
    ).toBeNull();
    expect(apiMock.agentSearchDirs).not.toHaveBeenCalled();

    apiMock.clearAgentDir.mockResolvedValue(undefined);
    await user.click(await screen.findByRole("button", { name: en.common.remove }));

    await waitFor(() => expect(apiMock.clearAgentDir).toHaveBeenCalledWith("gemini"));
    await waitFor(() => expect(onRedetect).toHaveBeenCalledTimes(1));
  });

  it("checks a picked directory at once, before anything is stored", async () => {
    const { user } = await openMenu(missing);
    await user.click(await screen.findByRole("menuitem", { name: "Gemini CLI" }));

    // The picker hands back a directory: the check runs there and then, so a
    // wrong one is answered while the user is still looking at the field.
    openMock.mockResolvedValue("/opt/gemini/bin");
    apiMock.verifyAgentDir.mockResolvedValue({
      path: "/opt/gemini/bin/gemini",
      version: "1.2.3",
    });

    await user.click(await screen.findByRole("button", { name: en.providers.browse }));

    await waitFor(() =>
      expect(apiMock.verifyAgentDir).toHaveBeenCalledWith("gemini", "/opt/gemini/bin"),
    );
    // What it found, named: the file, not just the version.
    expect(
      await screen.findByText(en.providers.dirOk.replace("{version}", "1.2.3")),
    ).toBeInTheDocument();
    expect(screen.getByText("/opt/gemini/bin/gemini")).toBeInTheDocument();
    // Nothing was written on the strength of a check: that is still Add.
    expect(apiMock.setAgentDir).not.toHaveBeenCalled();
    // And the "looked in N directories, and not found" list is gone: it was the
    // explanation for asking, and this answers it.
    expect(screen.queryByText(en.providers.searchedDirs.replace("{count}", "2"))).toBeNull();
  });

  it("says why a picked directory is no good, and forgets it when edited", async () => {
    const { user } = await openMenu(missing);
    await user.click(await screen.findByRole("menuitem", { name: "Gemini CLI" }));

    openMock.mockResolvedValue("/opt/nowhere");
    apiMock.verifyAgentDir.mockRejectedValue(new Error("no `gemini` in /opt/nowhere"));
    await user.click(await screen.findByRole("button", { name: en.providers.browse }));

    expect(await screen.findByText("no `gemini` in /opt/nowhere")).toBeInTheDocument();

    // Typing makes the verdict about the path that came before it, so it goes.
    await user.type(screen.getByRole("textbox", { name: en.providers.installDir }), "/bin");
    expect(screen.queryByText("no `gemini` in /opt/nowhere")).toBeNull();
  });

  it("keeps what the user typed when the screen re-renders around it", async () => {
    // The parent rebuilds the dialog's `agent` prop on every render — it carries
    // the live probe result — so a reset keyed on that object would clear the
    // field every time anything upstream moved. Clicking Check again is exactly
    // such a move, and it is the click a user makes with a path already typed.
    const onRedetect = vi
      .fn()
      .mockResolvedValue([{ agent: "gemini", installed: false, path: null }]);
    const { user, view } = await openMenu(missing, onRedetect);
    await user.click(await screen.findByRole("menuitem", { name: "Gemini CLI" }));

    const field = await screen.findByRole("textbox", { name: en.providers.installDir });
    await user.type(field, "/opt/gemini/bin");

    // A re-render, which is what the screen does the moment a probe answers —
    // it holds the detection list, and the dialog's `agent` prop is rebuilt from
    // it every time.
    view.rerender(
      <Providers
        onAdd={() => {}}
        onEdit={() => {}}
        agentDetect={[...missing]}
        onRedetect={onRedetect}
      />,
    );

    expect(field).toHaveValue("/opt/gemini/bin");

    // And a re-probe answers on top of that without clearing it either.
    await user.click(screen.getByRole("button", { name: en.providers.checkAgain }));
    await waitFor(() =>
      expect(screen.getByText(en.providers.recheckMissing)).toBeInTheDocument(),
    );
    expect(field).toHaveValue("/opt/gemini/bin");
  });

  it("reports a re-probe that comes back empty, instead of looking ignored", async () => {
    // The answer rides the return value, so the test hands it back the way App
    // does: the list the probe produced.
    const onRedetect = vi
      .fn()
      .mockResolvedValue([{ agent: "gemini", installed: false, path: null }]);
    const { user } = await openMenu(missing, onRedetect);
    await user.click(await screen.findByRole("menuitem", { name: "Gemini CLI" }));

    await user.click(await screen.findByRole("button", { name: en.providers.checkAgain }));

    await waitFor(() =>
      expect(screen.getByText(en.providers.recheckMissing)).toBeInTheDocument(),
    );
    // Still asking, then: the field and the list stay where they were.
    expect(screen.getByRole("textbox", { name: en.providers.installDir })).toBeInTheDocument();
  });

  it("says so when the probe did not run, rather than claiming it found nothing", async () => {
    const onRedetect = vi.fn().mockResolvedValue(null);
    const { user } = await openMenu(missing, onRedetect);
    await user.click(await screen.findByRole("menuitem", { name: "Gemini CLI" }));

    await user.click(await screen.findByRole("button", { name: en.providers.checkAgain }));

    await waitFor(() =>
      expect(screen.getByText(en.providers.recheckNoAnswer)).toBeInTheDocument(),
    );
    expect(screen.queryByText(en.providers.recheckMissing)).toBeNull();
  });

  it("asks the machine again, and says so when the agent turns up", async () => {
    const { user, onRedetect, view } = await openMenu(missing);
    await user.click(await screen.findByRole("menuitem", { name: "Gemini CLI" }));

    // The dialog offers the machine another chance — a tool installed since
    // launch is not something Kiwano can notice on its own.
    await user.click(await screen.findByRole("button", { name: en.providers.checkAgain }));
    await waitFor(() => expect(onRedetect).toHaveBeenCalledTimes(1));

    // What the probe answered arrives back as a prop (App owns the result): the
    // agent is installed, so there is nothing to point at. `rerender` is the
    // prop arriving.
    view.rerender(
      <Providers
        onAdd={() => {}}
        onEdit={() => {}}
        agentDetect={[{ agent: "gemini", installed: true, path: "/opt/gemini/bin/gemini" }]}
        onRedetect={onRedetect}
      />,
    );

    expect(await screen.findByText(en.providers.foundTitle)).toBeInTheDocument();
    // Where it turned up, in full — that is the question the user came with.
    expect(screen.getByText("/opt/gemini/bin/gemini")).toBeInTheDocument();
    // And the question is over: no directory to type, no declaration to make.
    expect(screen.queryByRole("textbox", { name: en.providers.installDir })).toBeNull();
    expect(screen.queryByText(en.providers.addAgentNote)).toBeNull();
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

  it("re-reads the row, so the cell shows what the click recorded", async () => {
    const user = await renderAll();
    apiMock.testProviderLatency.mockResolvedValue({
      provider_id: "deepseek",
      model: "deepseek-chat",
      latency_ms: 218,
      status: 200,
      error: null,
    });
    const reads = apiMock.listProviders.mock.calls.length;

    await user.click(await screen.findByRole("button", { name: en.providers.testLatency }));

    // The backend records the verdict as the provider's health, so the click is
    // also what makes the Status cell current — a stale "No answer" is exactly
    // what this button settles.
    await waitFor(() => expect(apiMock.listProviders.mock.calls.length).toBeGreaterThan(reads));
  });

  it("is on an agent's rows too, where it records the same verdict", async () => {
    // The test measures the *provider*, not the binding, so the same button
    // belongs on both tabs — and it writes one verdict, which is what keeps the
    // two tabs from disagreeing about what it found. Park, edit and delete stay
    // on the All tab: parking is global, and would drop the provider out of every
    // agent's route from a view scoped to one.
    apiMock.listProviders.mockResolvedValue([deepseek()]);
    apiMock.getAgentRoutes.mockResolvedValue([codexRoute(["deepseek"])]);
    apiMock.getSettings.mockResolvedValue(settingsWith(true, []));
    render(<Providers onAdd={() => {}} onEdit={() => {}} initialAgent="codex" />);
    const user = userEvent.setup();
    apiMock.testProviderLatency.mockResolvedValue({
      provider_id: "deepseek",
      model: "deepseek-chat",
      latency_ms: 218,
      status: 200,
      error: null,
    });
    const reads = apiMock.listProviders.mock.calls.length;

    await user.click(await screen.findByRole("button", { name: en.providers.testLatency }));

    await waitFor(() => expect(apiMock.testProviderLatency).toHaveBeenCalledWith("deepseek"));
    await waitFor(() => expect(apiMock.listProviders.mock.calls.length).toBeGreaterThan(reads));
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

/** The Status column's two measurements, which are not the same claim and must
    not be read as one: a provider's own round trips through the gateway, and —
    where it has had no traffic in the last day — the gateway's unsigned GET. */
describe("the Status cell", () => {
  /** One row per reading the cell can show. */
  async function renderRows() {
    apiMock.listProviders.mockResolvedValue([
      deepseek({ id: "traffic", health: { state: "ok", latency_ms: 1100, source: "traffic" } }),
      deepseek({
        id: "probe",
        health: { state: "ok", latency_ms: 18, source: "probe", checked_at: "2026-09-15T07:20:00Z" },
      }),
      deepseek({
        id: "silent",
        health: { state: "error", latency_ms: null, source: "probe", checked_at: "2026-09-15T07:20:00Z" },
      }),
      deepseek({
        id: "parked",
        enabled: false,
        health: { state: "off", latency_ms: null, note: "Disabled" },
      }),
      // A number the reader's own test produced: a real prompt, with the key.
      deepseek({
        id: "tested",
        health: { state: "ok", latency_ms: 218, source: "test", checked_at: "2026-09-15T11:38:00Z" },
      }),
      // Answered and refused the key. Reachable — the vendor is talking — so the
      // cell has to read as the key rather than as silence.
      deepseek({
        id: "refused",
        health: {
          state: "error",
          latency_ms: 60,
          source: "test",
          checked_at: "2026-09-15T11:38:00Z",
          error: "invalid API key",
        },
      }),
      // Enabled, never used, never probed: the cell says nothing, which is the
      // state of a provider added a minute ago.
      deepseek({ id: "fresh", health: { state: "idle", latency_ms: null } }),
    ]);
    apiMock.getAgentRoutes.mockResolvedValue([]);
    apiMock.getSettings.mockResolvedValue(settingsWith(true, []));
    render(<Providers onAdd={() => {}} onEdit={() => {}} />);
  }

  it("says which measurement it is showing, and what it proves", async () => {
    await renderRows();

    // The provider's own requests — one row, and the tooltip says which.
    expect(await screen.findByText("1.1s")).toBeInTheDocument();
    expect(screen.getAllByTitle(en.providers.latencyTraffic)).toHaveLength(1);

    // The prober's verdict, tooltipped with what it is and when it ran: an
    // unsigned request, so it is not the same claim as the number above.
    expect(screen.getByText("18ms").closest("[title]")).toHaveAttribute(
      "title",
      en.providers.latencyProbe.replace("{time}", "07:20"),
    );

    // An endpoint that did not answer is a fact, not a latency.
    expect(screen.getByText(en.providers.unreachable).closest("[title]")).toHaveAttribute(
      "title",
      en.providers.unreachableTitle.replace("{time}", "07:20"),
    );

    // The app's own test says so in its tooltip: a real prompt with the key,
    // which is a stronger claim than either of the gateway's.
    expect(screen.getByText("218ms").closest("[title]")).toHaveAttribute(
      "title",
      en.providers.latencyTest.replace("{time}", "11:38"),
    );

    // And a refusal reads as the key, not as silence — with the vendor's own
    // words in the tooltip, since that is what tells the two apart.
    expect(screen.getByText(en.providers.refused).closest("[title]")).toHaveAttribute(
      "title",
      en.providers.refusedTitle
        .replace("{error}", "invalid API key")
        .replace("{time}", "11:38"),
    );

    // Parked outranks both.
    expect(screen.getByText("Disabled")).toBeInTheDocument();
  });
});

describe("deleting a provider that is in a route", () => {
  /** codex's route, with the candidates named as the row would show them. */
  function routeOf(...providers: [string, string][]): AgentRoute {
    return {
      agent: "codex",
      strategy: "single",
      config: null,
      limits: [],
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

  it("says when it is done, because usually nothing moved", async () => {
    const user = userEvent.setup();
    apiMock.listProviders.mockResolvedValue([deepseek()]);
    apiMock.getAgentRoutes.mockResolvedValue([]);
    apiMock.getSettings.mockResolvedValue(settingsWith(true, []));
    render(<Providers onAdd={() => {}} onEdit={() => {}} />);
    const button = await screen.findByRole("button", { name: en.common.refresh });

    // At rest it is the reload glyph (lucide names the icon on the svg, which is
    // the button's only state handle — its label is the action).
    expect(button.querySelector("svg.lucide-refresh-cw")).not.toBeNull();

    await user.click(button);

    // Every half of the refresh is awaited together, so the acknowledgement
    // lands when the last of them does — and it is a flash, not a mode.
    await waitFor(() => expect(button.querySelector("svg.lucide-check")).not.toBeNull());
    await waitFor(
      () => expect(button.querySelector("svg.lucide-refresh-cw")).not.toBeNull(),
      { timeout: 3000 },
    );
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

// The agent's own ceilings, which live in the agent's settings dialog — not in
// the strategy panel, because they hold under every strategy, and not as one
// window, because an agent can hold several at once.
describe("the agent's own limit", () => {
  /** Open the agent's settings from the row above its table. */
  async function openSettings(user: ReturnType<typeof userEvent.setup>) {
    await user.click(
      await screen.findByLabelText(
        en.providers.agentSettingsFor.replace("{agent}", "Codex"),
      ),
    );
  }

  /** …and pick one of its tabs: everything is behind one, so a test that means
      the ceilings has to say so. */
  async function openTab(user: ReturnType<typeof userEvent.setup>, name: string) {
    await openSettings(user);
    await user.click(screen.getByRole("button", { name }));
  }

  /** The amount field of one window, named by its window's label — a screen may
      show several, so the window is part of the field's own name. */
  const amountFor = (windowLabel: string) =>
    screen.getByLabelText(`${en.strategy.limitAmountAria} · ${windowLabel}`);

  it("shows a row per stored window, and judges them together", async () => {
    const user = userEvent.setup();
    renderCodexTab({
      providers: [deepseek()],
      routes: [
        codexRoute(["deepseek"], [
          { period: "day", period_limit: 100, limit_unit: null },
          { period: "monthly", period_limit: 50, limit_unit: "CNY" },
        ]),
      ],
      codexTakenOver: true,
    });
    await openTab(user, en.strategy.limitLabel);

    // Both windows are there, each with its own amount — the pair is what the
    // gateway judges, so the pair is what the screen shows.
    expect(amountFor(en.strategy.limitPeriodDay)).toHaveValue(100);
    expect(amountFor(en.strategy.limitPeriodMonth)).toHaveValue(50);
    expect(screen.queryByText(en.strategy.limitNone)).toBeNull();
  });

  it("says no limit when there are no windows", async () => {
    const user = userEvent.setup();
    renderCodexTab({
      providers: [deepseek()],
      routes: [codexRoute(["deepseek"])],
      codexTakenOver: true,
    });
    await openTab(user, en.strategy.limitLabel);

    expect(screen.getByText(en.strategy.limitNone)).toBeInTheDocument();
  });

  it("adds a window and saves the whole set", async () => {
    const user = userEvent.setup();
    renderCodexTab({
      providers: [deepseek()],
      routes: [
        codexRoute(["deepseek"], [
          { period: "monthly", period_limit: 50, limit_unit: "CNY" },
        ]),
      ],
      codexTakenOver: true,
    });
    await openTab(user, en.strategy.limitLabel);

    await user.click(screen.getByRole("button", { name: en.strategy.limitAdd }));
    // The window it opens on is one the agent does not already have.
    await user.type(amountFor(en.strategy.limitPeriodDay), "25");
    await user.click(screen.getByRole("button", { name: en.common.save }));

    await waitFor(() =>
      expect(apiMock.setAgentLimits).toHaveBeenCalledWith("codex", [
        { period: "monthly", period_limit: 50, limit_unit: "CNY" },
        // Its one provider bills in USD, so that is the only money a new window
        // could be in — and the display currency (CNY in this fixture) is not
        // consulted at all.
        { period: "day", period_limit: 25, limit_unit: "USD" },
      ]),
    );
  });

  /// A ceiling is written in money that is being spent. The unit picker's
  /// options are therefore the currencies this agent's own providers bill in —
  /// what the routing actually charges — rather than the display currency,
  /// which is a preference about how numbers read.
  it("makes the reader pick a unit when its providers bill in more than one", async () => {
    const user = userEvent.setup();
    renderCodexTab({
      // Two providers, two currencies: nothing here can choose for the reader.
      providers: [deepseek(), deepseek({ id: "glm", name: "GLM", currency: "CNY" })],
      routes: [codexRoute(["deepseek"])],
      codexTakenOver: true,
    });
    await openTab(user, en.strategy.limitLabel);

    await user.click(screen.getByRole("button", { name: en.strategy.limitAdd }));
    await user.type(amountFor(en.strategy.limitPeriodDay), "25");
    await user.click(screen.getByRole("button", { name: en.common.save }));

    // A request count, which is what a ceiling starts as when there is a choice
    // to make.
    await waitFor(() =>
      expect(apiMock.setAgentLimits).toHaveBeenCalledWith("codex", [
        { period: "day", period_limit: 25, limit_unit: null },
      ]),
    );
  });

  it("keeps reading a stored unit its providers no longer name", async () => {
    const user = userEvent.setup();
    renderCodexTab({
      providers: [deepseek()], // bills in USD
      routes: [
        codexRoute(["deepseek"], [
          // Set while the agent still had a CNY provider, or from the CLI.
          { period: "monthly", period_limit: 50, limit_unit: "CNY" },
        ]),
      ],
      codexTakenOver: true,
    });
    await openTab(user, en.strategy.limitLabel);

    // The row is in CNY and has to keep saying so: a ceiling that had gone
    // blank would read as one that is not set, while the gateway goes on
    // enforcing it.
    expect(screen.getByText("CNY")).toBeInTheDocument();
  });

  it("removes a window, and saving writes what is left", async () => {
    const user = userEvent.setup();
    renderCodexTab({
      providers: [deepseek()],
      routes: [
        codexRoute(["deepseek"], [
          { period: "day", period_limit: 100, limit_unit: null },
          { period: "monthly", period_limit: 50, limit_unit: "CNY" },
        ]),
      ],
      codexTakenOver: true,
    });
    await openTab(user, en.strategy.limitLabel);

    await user.click(screen.getAllByRole("button", { name: en.strategy.limitRemoveAria })[0]);
    await user.click(screen.getByRole("button", { name: en.common.save }));

    await waitFor(() =>
      expect(apiMock.setAgentLimits).toHaveBeenCalledWith("codex", [
        { period: "monthly", period_limit: 50, limit_unit: "CNY" },
      ]),
    );
  });

  it("is on a user-defined agent's dialog too, where it is the only way in", async () => {
    const user = userEvent.setup();
    const longTasks: CustomAgent = {
      id: "long-tasks-3f9a",
      label: "Long tasks",
      note: null,
      protocol: "openai",
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
      screen.getByLabelText(en.providers.agentSettingsFor.replace("{agent}", longTasks.label)),
    );
    // Its dialog has the same two subjects; this one opens on the credentials.
    await user.click(screen.getByRole("button", { name: en.strategy.limitLabel }));

    await user.click(screen.getByRole("button", { name: en.strategy.limitAdd }));
    await user.type(amountFor(en.strategy.limitPeriodDay), "5");
    await user.click(screen.getByRole("button", { name: en.common.save }));

    await waitFor(() =>
      expect(apiMock.setAgentLimits).toHaveBeenCalledWith(longTasks.id, [
        { period: "day", period_limit: 5, limit_unit: null },
      ]),
    );
  });
});

// A built-in agent's tab names itself, and names the file behind it. The strip
// above is icons only, so without this row the page has no agent name on it at
// all — and no way to find the config a takeover rewrites, which is what someone
// reaches for by hand when the takeover is the problem.
describe("taking an agent over", () => {
  it("shows why the backend refused, instead of leaving a spin and no reason", async () => {
    // The refusal a config-dir variable can produce, and the one codex's gates
    // already produced: the attempt changes nothing, so the reason is the only
    // thing the click can tell the user.
    apiMock.setTakeover.mockRejectedValue(
      new Error("XDG_CONFIG_HOME is set to `rel` — not an absolute path."),
    );
    apiMock.listProviders.mockResolvedValue([deepseek()]);
    apiMock.getAgentRoutes.mockResolvedValue([]);
    apiMock.getSettings.mockResolvedValue(settingsWith(false, []));
    render(
      <Providers
        onAdd={() => {}}
        onEdit={() => {}}
        initialAgent="codex"
        agentDetect={codexInstalled}
      />,
    );

    await userEvent.setup().click(
      await screen.findByRole("button", { name: en.providers.enableKiwano }),
    );

    expect(
      await screen.findByText("XDG_CONFIG_HOME is set to `rel` — not an absolute path."),
    ).toBeInTheDocument();
  });
});

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

  it("is not there before the agent is taken over — the tab offers to enable it", async () => {
    renderCodexTab({
      providers: [deepseek()],
      routes: [codexRoute(["deepseek"])],
      codexTakenOver: false,
    });

    // The row is about an agent Kiwano is already routing; the file it names is
    // the agent's own until then, and getting to it is the onboarding panel's job.
    expect(
      await screen.findByText(en.providers.startManaging.replace("{agent}", "Codex")),
    ).toBeInTheDocument();
    expect(
      screen.queryByLabelText(en.providers.agentSettingsFor.replace("{agent}", "Codex")),
    ).toBeNull();
  });

  it("keeps the files behind their own tab", async () => {
    const user = userEvent.setup();
    renderCodexTab({
      providers: [deepseek()],
      routes: [codexRoute(["deepseek"])],
      codexTakenOver: true,
    });
    await user.click(
      await screen.findByLabelText(en.providers.agentSettingsFor.replace("{agent}", "Codex")),
    );

    // Two readings, one showing: the files are what it opens on, and the ceiling
    // is not in the DOM at all until its tab is picked.
    expect(screen.getByText("~/.codex/config.toml")).toBeInTheDocument();
    // The protocol is one of the facts on this tab, and it is *stated*: codex's
    // own clients speak `wire_api = "responses"`, so there is no picker here to
    // contradict them with.
    expect(screen.getByText(en.providers.agentProtocol)).toBeInTheDocument();
    expect(screen.getByText(en.addProvider.protoOpenai)).toBeInTheDocument();
    expect(screen.queryByRole("combobox", { name: en.providers.agentProtocol })).toBeNull();
    expect(screen.getByText("~/.codex/auth.json")).toBeInTheDocument();
    expect(screen.getByText(en.providers.agentConfigFilesNote)).toBeInTheDocument();
    expect(screen.queryByText(en.strategy.limitNone)).toBeNull();

    await user.click(screen.getByRole("button", { name: en.strategy.limitLabel }));

    expect(screen.getByText(en.strategy.limitNone)).toBeInTheDocument();
    expect(screen.queryByText("~/.codex/auth.json")).toBeNull();
  });

  it("reads out the key a client is pointed at with, and turns the takeover off here", async () => {
    apiMock.setTakeover.mockResolvedValue(undefined);
    const user = userEvent.setup();
    renderCodexTab({
      providers: [deepseek()],
      routes: [codexRoute(["deepseek"])],
      codexTakenOver: true,
    });

    await user.click(
      await screen.findByLabelText(en.providers.agentSettingsFor.replace("{agent}", "Codex")),
    );
    // The gateway's key for this agent. It was a column in Settings' list of
    // every agent; it is one fact about this one, and this is where the agent's
    // own facts are.
    expect(screen.getByText("kw-ag-codex-test")).toBeInTheDocument();

    // It rewrites the agent's config back, so it takes two clicks: the first
    // only arms it.
    await user.click(screen.getByRole("button", { name: en.providers.agentDisable }));
    expect(apiMock.setTakeover).not.toHaveBeenCalled();

    await user.click(screen.getByRole("button", { name: en.providers.agentDisableConfirm }));

    await waitFor(() => expect(apiMock.setTakeover).toHaveBeenCalledWith("codex", false));
    // The dialog is about a takeover that no longer exists, so it goes with it —
    // and the tab is back to the onboarding that can turn one on again.
    await waitFor(() => expect(screen.queryByText(en.providers.agentConfigFilesNote)).toBeNull());
  });

  it("keeps that dialog open when the restore is refused, and says why", async () => {
    apiMock.setTakeover.mockRejectedValue(new Error("restore failed"));
    const user = userEvent.setup();
    renderCodexTab({
      providers: [deepseek()],
      routes: [codexRoute(["deepseek"])],
      codexTakenOver: true,
    });

    await user.click(
      await screen.findByLabelText(en.providers.agentSettingsFor.replace("{agent}", "Codex")),
    );
    await user.click(screen.getByRole("button", { name: en.providers.agentDisable }));
    await user.click(screen.getByRole("button", { name: en.providers.agentDisableConfirm }));

    // Codex is still taken over — nothing was restored — so the reason has to
    // land here: with the agent still routed, this dialog is the only place left
    // that can show one.
    expect(await screen.findByText("restore failed")).toBeInTheDocument();
    expect(screen.getByText(en.providers.agentConfigFilesNote)).toBeInTheDocument();
  });

  it("is not on the all-agents tab, and not on a user-defined agent's", async () => {
    const user = userEvent.setup();
    const longTasks: CustomAgent = {
      id: "long-tasks-3f9a",
      label: "Long tasks",
      note: null,
      protocol: "openai",
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

    // A user-defined agent has no config file behind it, so its tab has no name +
    // path row. Its gear is still there — that is where its settings live — and
    // opens the credentials rather than a file list.
    await user.click(await screen.findByRole("button", { name: longTasks.label }));
    expect(screen.queryByText(/~\//)).toBeNull();
    await user.click(
      screen.getByLabelText(en.providers.agentSettingsFor.replace("{agent}", longTasks.label)),
    );
    expect(screen.getByText(en.providers.accessEndpoint)).toBeInTheDocument();
    expect(screen.queryByText(en.providers.agentConfigFilesNote)).toBeNull();
  });
});

// The usage column's own change signal. The gateway pushes a tick per recorded
// request and the screen re-reads, so a re-read is not itself news — this is
// what says *which* row moved, judged on the numbers rather than on the read.
describe("a usage cell whose numbers moved", () => {
  /** Enough of a summary for the cell to draw its numbers. */
  const usage = (requests: number): UsageSummary => ({
    requests,
    input_tokens: 1000,
    output_tokens: 100,
    cache_read_tokens: 0,
    cache_creation_tokens: 0,
    cost: 1,
    cost_currency: "USD",
    latency_ms: null,
    quota: null,
    spark: null,
  });

  const cells = (root: HTMLElement) => [...root.querySelectorAll(".usage-cell")];

  it("tints the row whose numbers moved, and leaves the rest alone", async () => {
    const user = userEvent.setup();
    apiMock.testProviderLatency.mockResolvedValue(undefined);
    const { container } = renderCodexTab({
      providers: [
        deepseek({ usage: usage(10) }),
        deepseek({ id: "kimi", name: "Kimi", usage: usage(20) }),
      ],
      routes: [codexRoute(["deepseek", "kimi"])],
      codexTakenOver: true,
    });
    await screen.findByText("Kimi");
    expect(cells(container)).toHaveLength(2);

    // One re-read, in which one provider's numbers moved and the other's did
    // not. The latency test is the trigger because it re-reads everything
    // without changing which rows are on screen.
    apiMock.listProviders.mockResolvedValue([
      deepseek({ usage: usage(10) }),
      deepseek({ id: "kimi", name: "Kimi", usage: usage(21) }),
    ]);
    await user.click((await screen.findAllByLabelText(en.providers.testLatency))[1]);

    await waitFor(() => expect(cells(container)[1].className).toContain("changed"));
    expect(cells(container)[0].className).not.toContain("changed");
  });

  it("leaves a cell that was simply empty alone when its first read lands", async () => {
    const user = userEvent.setup();
    apiMock.testProviderLatency.mockResolvedValue(undefined);
    const { container } = renderCodexTab({
      providers: [deepseek()], // no usage at all: the cell draws nothing
      routes: [codexRoute(["deepseek"])],
      codexTakenOver: true,
    });
    await screen.findByText("DeepSeek");

    // The numbers arriving is not a number that moved — the cell was empty
    // because there was nothing to show, and a tint there would fire on every
    // screen the moment its data landed.
    apiMock.listProviders.mockResolvedValue([deepseek({ usage: usage(3) })]);
    await user.click(await screen.findByLabelText(en.providers.testLatency));

    await waitFor(() => expect(apiMock.listProviders).toHaveBeenCalledTimes(2));
    expect(cells(container)[0].className).not.toContain("changed");
  });
});

// The quota strategy's over-threshold state: the configured first backup is
// *first in line*, not in use — which backup actually serves depends on the
// gateway's breakers at request time, so the badge is a config claim with a
// different mark ("Fallback") and the over-threshold primary claims nothing.
describe("the quota fallback badge", () => {
  const over = () => [
    deepseek({
      id: "quota-primary",
      name: "Quota Primary",
      agents: ["codex"],
      serving_agents: [],
      is_current: false,
    }),
    deepseek({
      id: "quota-backup",
      name: "Quota Backup",
      agents: ["codex"],
      serving_agents: [],
      is_current: false,
      fallback_agents: ["codex"],
    }),
  ];

  it("reads as Fallback, not In use, on the All tab", async () => {
    apiMock.listProviders.mockResolvedValue(over());
    apiMock.getAgentRoutes.mockResolvedValue([codexRoute(["quota-primary", "quota-backup"])]);
    apiMock.getSettings.mockResolvedValue(settingsWith(true, []));
    render(<Providers onAdd={() => {}} onEdit={() => {}} />);

    expect(await screen.findByText("Quota Backup")).toBeInTheDocument();
    const chip = screen.getByText(en.providers.quotaFallback);
    // First in line behind an exhausted primary — a configured state, which is
    // why the honest claim is about the config and the tooltip says what it is.
    expect(chip).toHaveAttribute("title", en.providers.quotaFallbackTitle);
    // Neither row claims to be serving: the primary is over its threshold, and
    // the backup is not serving yet either.
    expect(screen.queryAllByText(en.providers.inUse)).toHaveLength(0);
  });

  it("badges the same claim on an agent tab", async () => {
    apiMock.listProviders.mockResolvedValue(over());
    apiMock.getAgentRoutes.mockResolvedValue([codexRoute(["quota-primary", "quota-backup"])]);
    apiMock.getSettings.mockResolvedValue(settingsWith(true, []));
    render(<Providers onAdd={() => {}} onEdit={() => {}} initialAgent="codex" />);

    expect(await screen.findByText(en.providers.quotaFallback)).toBeInTheDocument();
    expect(screen.queryByText(en.providers.inUse)).toBeNull();
  });
});

// A provider read that failed. `refetch` absorbs every branch's failure — a
// mutation or the app's ⟳ that calls it fire-and-forget must never hit an
// unhandled rejection — and the screen says what went wrong where the reader
// can see it: the initial failure takes the empty state (endless "Loading…"
// would be waiting on a read that is not coming), a later failure keeps the
// stale rows under a banner.
describe("a failed provider read", () => {
  it("shows the failure and a retry instead of loading forever", async () => {
    apiMock.listProviders.mockRejectedValue(new Error("invoke failed: gateway unreachable"));
    apiMock.getAgentRoutes.mockResolvedValue([]);
    apiMock.getSettings.mockResolvedValue(settingsWith(true, []));
    render(<Providers onAdd={() => {}} onEdit={() => {}} />);

    expect(await screen.findByText(en.common.loadFailed)).toBeInTheDocument();
    // The raw failure never reads as a sentence, so it lives in the tooltip —
    // on the alert strip itself, which is where the title attaches.
    expect(screen.getByRole("alert")).toHaveAttribute(
      "title",
      expect.stringContaining("gateway unreachable"),
    );

    // Retry recovers once the read can be made again.
    apiMock.listProviders.mockResolvedValue([deepseek()]);
    await userEvent.setup().click(screen.getByRole("button", { name: en.common.retry }));
    expect(await screen.findByText("DeepSeek")).toBeInTheDocument();
  });

  it("keeps stale rows, and says the re-read failed, rather than dropping them", async () => {
    apiMock.listProviders.mockResolvedValue([deepseek()]);
    apiMock.getAgentRoutes.mockResolvedValue([]);
    apiMock.getSettings.mockResolvedValue(settingsWith(true, []));
    render(<Providers onAdd={() => {}} onEdit={() => {}} />);
    const user = userEvent.setup();
    expect(await screen.findByText("DeepSeek")).toBeInTheDocument();

    // The next read fails: the row stays (stale beats blank), the failure is said.
    apiMock.listProviders.mockRejectedValue(new Error("invoke failed"));
    await user.click(screen.getByRole("button", { name: en.common.refresh }));
    await waitFor(() => expect(screen.getByText(en.common.loadFailed)).toBeInTheDocument());
    expect(screen.getByText("DeepSeek")).toBeInTheDocument();
  });
});

describe("the cache column", () => {
  const usage = (over: Partial<NonNullable<Provider["usage"]>> = {}) => ({
    requests: 796,
    input_tokens: 5_400_000,
    cache_read_tokens: 4_300_000,
    cache_creation_tokens: 600_000,
    output_tokens: 900_000,
    cost: null,
    latency_ms: 1100,
    quota: null,
    spark: null,
    ...over,
  });

  it("rates the input-side tokens served from cache", async () => {
    // 4.3M reads out of 5.4M new + 4.3M read + 0.6M written = 42%: the writes
    // are in the denominator, so leaving them out would read 44%.
    renderCodexTab({
      providers: [deepseek({ usage: usage() })],
      routes: [codexRoute(["deepseek"])],
      codexTakenOver: true,
    });
    expect(await screen.findByText("42%")).toBeInTheDocument();
    expect(screen.getByText(en.providers.colCache)).toBeInTheDocument();
  });

  it("dashes a window with no input-side tokens instead of calling it 0%", async () => {
    renderCodexTab({
      providers: [
        deepseek({
          usage: usage({ input_tokens: 0, cache_read_tokens: 0, cache_creation_tokens: 0 }),
        }),
      ],
      routes: [codexRoute(["deepseek"])],
      codexTakenOver: true,
    });
    expect(await screen.findByTitle(en.providers.cacheNoData)).toBeInTheDocument();
    expect(screen.queryByText("0%")).toBeNull();
  });
});
