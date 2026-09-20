// The Add-provider dialog's two modes, from the Custom side.
//
// A provider typed in by hand declares three things a catalog entry would
// otherwise declare for it: which protocol it speaks, which endpoints it answers
// on, and the currency its spending limit is in. Those three are read-only while
// an entry owns the form — the entry's published prices are what the limit is
// measured against — and the user's own otherwise. The tests below are that
// sentence, both ways round, asserted on what the backend is finally handed.
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { en } from "@/i18n/en";
import type { CatalogEntry } from "@/api/types";

import AddProviderModal from "./AddProviderModal";

const { apiMock } = vi.hoisted(() => ({
  apiMock: {
    getCurrencyMeta: vi.fn(),
    getSettings: vi.fn(),
    listCatalog: vi.fn(),
    listApiKeys: vi.fn(),
    addProvider: vi.fn(),
    updateProvider: vi.fn(),
    testEndpoint: vi.fn(),
    listModels: vi.fn(),
  },
}));

vi.mock("../api/client", () => ({ api: apiMock }));

/** A shelf entry, as the Models page would hand one to the dialog. */
const entry: CatalogEntry = {
  id: "moonshot",
  name: "Moonshot",
  logo_color: "#111111",
  tag: "official",
  website: "https://moonshot.example",
  rating: 4.5,
  endpoint: "https://api.moonshot.example",
  protocol: "anthropic",
  desc: "",
  currency: "CNY",
  billing: "payg",
  added: false,
  models: ["kimi-k2"],
};

beforeEach(() => {
  vi.clearAllMocks();
  apiMock.getCurrencyMeta.mockResolvedValue({
    preferred: "CNY",
    currencies: ["USD", "CNY", "EUR"],
    exchange_rates: { USD: 1, CNY: 7, EUR: 0.9 },
  });
  apiMock.getSettings.mockResolvedValue({ hub_url: "https://hub.example" });
  apiMock.listCatalog.mockResolvedValue({ entries: [entry], total: 1 });
  apiMock.listApiKeys.mockResolvedValue([]);
  apiMock.addProvider.mockResolvedValue({});
});

/** Open on the Custom tab, and fill what the save button requires. */
async function openCustom() {
  const user = userEvent.setup();
  render(
    <AddProviderModal
      open
      preset={null}
      edit={null}
      preferredCurrency="CNY"
      onClose={() => {}}
      onSaved={() => {}}
    />,
  );
  await user.click(screen.getByText(en.addProvider.custom));
  await user.type(screen.getByLabelText(en.addProvider.name), "My Provider");
  await user.type(
    screen.getAllByLabelText(en.addProvider.endpointUrl)[0],
    "https://api.example.com",
  );
  return user;
}

/** What the save button hands the backend. */
async function savedPayload(user: ReturnType<typeof userEvent.setup>) {
  await user.click(screen.getByRole("button", { name: en.addProvider.saveEnable }));
  await waitFor(() => expect(apiMock.addProvider).toHaveBeenCalled());
  return apiMock.addProvider.mock.calls[0][0];
}

/** The Advanced fold holds timeout/retries/headers behind one button. */
async function openAdvanced(user: ReturnType<typeof userEvent.setup>) {
  await user.click(screen.getByRole("button", { name: en.addProvider.advanced }));
}

/** Each header becomes a name/value pair of inputs; `i` is the row index. */
function headerInputs(i: number): { name: HTMLElement; value: HTMLElement } {
  const placeholders = screen.getAllByPlaceholderText(en.addProvider.headerValuePlaceholder);
  return {
    name: screen.getAllByPlaceholderText(en.addProvider.headerNamePlaceholder)[i],
    value: placeholders[i],
  };
}

describe("custom headers that name an auth header", () => {
  it("says when a custom header overrides the injected credential", async () => {
    const user = await openCustom();
    await openAdvanced(user);

    // A benign header draws no warning — most rows do not touch credentials.
    await user.click(screen.getByRole("button", { name: en.addProvider.addHeader }));
    await user.type(headerInputs(0).name, "X-Custom");
    await user.type(headerInputs(0).value, "hello");
    expect(screen.queryByText(en.addProvider.customHeadersAuthWarning.replace("{name}", "X-Custom"))).toBeNull();

    // The nudge appears as soon as an auth header is named, whatever the case:
    // HTTP header names are case-insensitive, so the check has to be too.
    await user.clear(headerInputs(0).name);
    await user.type(headerInputs(0).name, "x-api-key");
    expect(
      await screen.findByText(en.addProvider.customHeadersAuthWarning.replace("{name}", "x-api-key")),
    ).toBeInTheDocument();
    // …and goes away with the header that earned it.
    await user.click(screen.getByRole("button", { name: en.addProvider.removeHeader }));
    expect(
      screen.queryByText(en.addProvider.customHeadersAuthWarning.replace("{name}", "x-api-key")),
    ).toBeNull();
  });

  it("sends the override through when the user keeps it", async () => {
    const user = await openCustom();
    await openAdvanced(user);

    await user.click(screen.getByRole("button", { name: en.addProvider.addHeader }));
    await user.type(headerInputs(0).name, "authorization");
    await user.type(headerInputs(0).value, "Bearer sk-user");
    const payload = await savedPayload(user);

    // The headers travel as part of the advanced block, beside timeout/retries.
    expect(payload.advanced.headers).toEqual({ authorization: "Bearer sk-user" });
  });
});

describe("adding a provider by hand", () => {
  it("lets the user pick the protocol", async () => {
    const user = await openCustom();

    // The endpoint row is where the protocol lives, and it is the only control
    // for it — the block that used to summarise the rows was removed.
    await user.click(screen.getByRole("combobox", { name: en.addProvider.protocol }));
    await user.click(await screen.findByRole("option", { name: "Anthropic" }));

    expect((await savedPayload(user)).protocol).toBe("anthropic");
  });

  it("lets the user add an endpoint, on the protocol nothing covers yet", async () => {
    const user = await openCustom();

    await user.click(screen.getByRole("button", { name: en.addProvider.addEndpoint }));
    await user.type(
      screen.getAllByLabelText(en.addProvider.endpointUrl)[1],
      "https://api.example.com/anthropic",
    );

    // The primary is OpenAI, so the row that appears is the other protocol —
    // a second row on the same one would be a duplicate the save drops.
    expect((await savedPayload(user)).endpoints).toEqual([
      { protocol: "anthropic", endpoint: "https://api.example.com/anthropic" },
    ]);
  });

  it("lets the user take an endpoint away again", async () => {
    const user = await openCustom();

    await user.click(screen.getByRole("button", { name: en.addProvider.addEndpoint }));
    await user.type(
      screen.getAllByLabelText(en.addProvider.endpointUrl)[1],
      "https://api.example.com/anthropic",
    );
    await user.click(screen.getByRole("button", { name: en.addProvider.removeEndpoint }));

    expect((await savedPayload(user)).endpoints).toEqual([]);
  });

  it("lets the user choose the currency the limit is in", async () => {
    const user = await openCustom();

    // Seeded from the user's own currency, since no entry stated one.
    const currency = screen.getByRole("combobox", { name: en.addProvider.spendingLimit });
    expect(currency).toHaveTextContent("CNY");

    await user.click(currency);
    await user.click(await screen.findByRole("option", { name: "EUR" }));
    await user.type(screen.getByPlaceholderText("50"), "20");

    const payload = await savedPayload(user);
    expect(payload.billing_config).toMatchObject({ limit_value: 20, limit_unit: "EUR" });
  });
});

describe("prices a hand-added provider declares", () => {
  it("carries the rates the user typed, in the limit's currency", async () => {
    const user = await openCustom();

    await user.click(screen.getByRole("button", { name: en.addProvider.addPrice }));
    await user.type(screen.getByLabelText(en.addProvider.priceModel), "kimi-k2");
    await user.type(screen.getAllByLabelText(en.addProvider.priceIn)[0], "1.5");
    await user.type(screen.getAllByLabelText(en.addProvider.priceOut)[0], "6");

    // Blank cache rates are sent as zeros rather than dropped: that is the
    // backend's reading of a rate nobody stated, and the note under the rows
    // says so.
    expect((await savedPayload(user)).prices).toEqual({
      currency: "CNY",
      models: [
        {
          model_id: "kimi-k2",
          input: "1.5",
          output: "6",
          cache_read: "0",
          cache_creation: "0",
        },
      ],
    });
  });

  it("offers them on pay as you go only", async () => {
    const user = await openCustom();
    const addPrice = () => screen.queryByRole("button", { name: en.addProvider.addPrice });
    expect(addPrice()).not.toBeNull();

    // A plan is billed by the window, not by the token; the rates would describe
    // a charge this provider does not make. And the save stops speaking about
    // them rather than clearing what is stored: the section is simply not there
    // for the user to have said anything.
    await user.click(screen.getByRole("button", { name: en.addProvider.billingPlan }));
    expect(addPrice()).toBeNull();
    expect((await savedPayload(user)).prices).toBeUndefined();

    await user.click(screen.getByRole("button", { name: en.addProvider.billingUnlimited }));
    expect(addPrice()).toBeNull();
  });

  it("refuses a rate that is not a number of zero or more", async () => {
    const user = await openCustom();
    await user.click(screen.getByRole("button", { name: en.addProvider.addPrice }));
    await user.type(screen.getByLabelText(en.addProvider.priceModel), "kimi-k2");
    await user.type(screen.getAllByLabelText(en.addProvider.priceIn)[0], "-3");

    // Refused rather than dropped: the price table parses these when it costs a
    // request, so the value that reached it would be every request's cost.
    expect(screen.getByText(en.addProvider.priceInvalid)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: en.addProvider.saveEnable })).toBeDisabled();
  });

  it("drops the row whose model was left blank", async () => {
    const user = await openCustom();
    await user.click(screen.getByRole("button", { name: en.addProvider.addPrice }));
    await user.type(screen.getAllByLabelText(en.addProvider.priceIn)[0], "1.5");

    // The id is the key the price is looked up by, so a row without one is not a
    // price — the same reason a blank endpoint row is dropped on save.
    expect((await savedPayload(user)).prices).toEqual({ currency: "CNY", models: [] });
  });
});

describe("editing a provider that declares prices", () => {
  it("reads them back into the boxes and sends the currency back", async () => {
    const user = userEvent.setup();
    render(
      <AddProviderModal
        open
        preset={null}
        edit={{
          id: "manual-1a2b3c",
          name: "Manual",
          logo_char: "M",
          logo_color: "#555555",
          endpoint: "api.example.com",
          protocol: "openai",
          endpoint_note: "OpenAI-compatible",
          billing: "payg",
          limit_unit: "CNY",
          currency: "CNY",
          prices: {
            currency: "CNY",
            models: [
              {
                model_id: "kimi-k2",
                input: "1.5",
                output: "6",
                cache_read: "0",
                cache_creation: "0",
              },
            ],
          },
          enabled: true,
          agents: [],
          serving_agents: [],
          is_current: false,
          health: { state: "idle", latency_ms: null },
          usage: null,
        }}
        preferredCurrency="CNY"
        onClose={() => {}}
        onSaved={() => {}}
      />,
    );

    expect(await screen.findByLabelText(en.addProvider.priceModel)).toHaveValue("kimi-k2");
    expect(screen.getAllByLabelText(en.addProvider.priceOut)[0]).toHaveValue("6");

    await user.click(screen.getByRole("button", { name: en.common.save }));
    await waitFor(() => expect(apiMock.updateProvider).toHaveBeenCalled());
    expect(apiMock.updateProvider.mock.calls[0][1].prices).toEqual({
      currency: "CNY",
      models: [
        {
          model_id: "kimi-k2",
          input: "1.5",
          output: "6",
          cache_read: "0",
          cache_creation: "0",
        },
      ],
    });
  });
});

describe("adding a provider from the catalog", () => {
  it("keeps the entry's protocol, endpoints and currency out of the user's hands", async () => {
    const user = userEvent.setup();
    render(
      <AddProviderModal
        open
        preset={entry}
        edit={null}
        preferredCurrency="CNY"
        onClose={() => {}}
        onSaved={() => {}}
      />,
    );

    // Its prices are what the limit is measured against, so the currency is the
    // entry's; its endpoints are what the entry publishes, so there is nothing to
    // add or drop; and the protocol is a badge rather than a picker.
    expect(await screen.findByLabelText(en.addProvider.name)).toHaveValue("Moonshot");
    expect(
      screen.getByRole("combobox", { name: en.addProvider.spendingLimit }),
    ).toBeDisabled();
    expect(screen.queryByRole("button", { name: en.addProvider.addEndpoint })).toBeNull();
    expect(screen.queryByRole("combobox", { name: en.addProvider.protocol })).toBeNull();
    expect(screen.queryByRole("button", { name: en.addProvider.removeEndpoint })).toBeNull();
    // Prices too: the entry prices the models it publishes, so there is nothing
    // for the user to declare while it owns the form — and the save says nothing
    // about them rather than declaring an empty table.
    expect(screen.queryByRole("button", { name: en.addProvider.addPrice })).toBeNull();
    await user.click(screen.getByRole("button", { name: en.addProvider.saveEnable }));
    await waitFor(() => expect(apiMock.addProvider).toHaveBeenCalled());
    expect(apiMock.addProvider.mock.calls[0][0].prices).toBeUndefined();

    // Not a picker, but still said: the endpoint row carries its protocol, which
    // is what the block above it used to repeat as a pair of lit badges.
    expect(screen.getByText("Anthropic")).toBeInTheDocument();
  });
});
