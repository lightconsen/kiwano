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

describe("adding a provider by hand", () => {
  it("lets the user pick the protocol", async () => {
    const user = await openCustom();

    // The badge row above the endpoints is derived from these rows, so the
    // picker is the authoritative control it always claimed to be.
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

describe("adding a provider from the catalog", () => {
  it("keeps the entry's protocol, endpoints and currency out of the user's hands", async () => {
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
  });
});
