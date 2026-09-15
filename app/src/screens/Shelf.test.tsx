// The shelf's two price variants, as the reader meets them: a switch between the
// peak and off-peak rates, and the step-up a long request pays.
//
// Which rate is *in force* is the gateway's answer and is not decided here — the
// shelf shows both and says which is which, on the vendor's clock. That makes
// these assertions about words and numbers on a cell: a window written in the
// reader's timezone, a discount with no window, or a step-up that only one of the
// two screens knows about would all read as a normal row.
import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { en } from "@/i18n/en";
import type { CatalogEntry, CatalogList, ModelPrice } from "@/api/types";
import { fmtMoney } from "../lib/format";

import Shelf from "./Shelf";
import { ReloadRegistryProvider } from "../lib/reload";

const { apiMock } = vi.hoisted(() => ({
  apiMock: {
    listCatalog: vi.fn(),
    listModelPrices: vi.fn(),
    getSettings: vi.fn(),
    updateSettings: vi.fn(),
    syncHub: vi.fn(),
    openUrl: vi.fn(),
  },
}));

vi.mock("../api/client", () => ({ api: apiMock }));

/** DeepSeek's published shape: the row's own rates are the peak ones (09:00–12:00
    and 14:00–18:00 on weekdays, Beijing), the off-peak block is half of them, and
    a request over 512k pays a stepped-up pair — with no schedule of its own. */
const PRICE = {
  model_id: "deepseek-chat",
  display_name: "DeepSeek Chat",
  input: "9",
  output: "27",
  currency: "CNY",
  off_peak: { in: "4.5", out: "13.5", cache_read: "0.9", cache_creation: "0" },
  peak_hours: {
    tz_offset: 480,
    windows: [{ days: ["mon", "tue", "wed", "thu", "fri"], start: "09:00", end: "12:00" }],
  },
  long_context: {
    over: 512_000,
    // `in`/`out`, the published spelling the row's own rates and the off-peak
    // block both use (the mirror's `ModelPrice` is the one that renames them).
    in: "4.20",
    out: "16.80",
    cache_read: "0.84",
    cache_creation: "0",
  },
};

function entry(id: string, name: string, over: Partial<CatalogEntry> = {}): CatalogEntry {
  return {
    id,
    name,
    logo_color: "#4D6BFE",
    tag: "official",
    rating: 4.8,
    endpoint: `https://api.${id}.com`,
    protocol: "openai",
    billing: "payg",
    added: false,
    models: ["deepseek-chat"],
    ...over,
  };
}

const catalog = (entries: CatalogEntry[]): CatalogList => ({ total: entries.length, entries });

function modelPrice(over: Partial<ModelPrice> = {}): ModelPrice {
  return {
    provider_id: "deepseek",
    model_id: "deepseek-chat",
    display_name: "DeepSeek Chat",
    input: "9",
    output: "27",
    cache_read: "0.9",
    cache_creation: "0",
    currency: "CNY",
    ...over,
  };
}

/** `in ¥9 / out ¥27` — the cell's own wording, from the same dictionary *and*
    the same money formatter, so what is asserted here is the composition rather
    than a second opinion about how ¥9 prints. */
const rates = (input: string, output: string) =>
  `${en.shelf.priceIn} ${fmtMoney(Number(input), "CNY")} / ${en.shelf.priceOut} ${fmtMoney(Number(output), "CNY")}`;

/** The cell's whole text: the representative model, then its pair of rates. */
const cell = (model: string, input: string, output: string) =>
  `${model} · ${rates(input, output)}`;

async function renderShelf(entries: CatalogEntry[], prices: ModelPrice[] = []) {
  apiMock.listCatalog.mockResolvedValue(catalog(entries));
  apiMock.listModelPrices.mockResolvedValue(prices);
  render(<Shelf onAdd={() => {}} />);
  await screen.findByText(entries[0].name);
  return screen.getByText(entries[0].name).closest("tr") as HTMLElement;
}

beforeEach(() => {
  vi.clearAllMocks();
  apiMock.getSettings.mockResolvedValue({
    shelf_sort: null,
    shelf_view: null,
    hub_url: "https://hub.example",
  });
  apiMock.updateSettings.mockResolvedValue({});
});

/** What the status bar's ⟳ does to a screen: read the data again, keep every
 *  choice the reader made. The button used to remount the screen — a changing
 *  `key` — which threw the chip, the search box and the sort away with it, so
 *  "refresh" behaved like "leave and come back". The screen now registers how to
 *  re-read itself (`lib/reload.ts`) and the button calls that. */
describe("the app's own reload", () => {
  it("re-reads the data and keeps what the reader had chosen", async () => {
    const user = userEvent.setup();
    let reload: (() => Promise<unknown>) | null = null;
    const register = (fn: () => Promise<unknown>) => {
      reload = fn;
      return () => {
        reload = null;
      };
    };
    apiMock.listCatalog.mockResolvedValue(
      catalog([entry("deepseek", "DeepSeek"), entry("kimi", "Kimi", { tag: "third" })]),
    );
    apiMock.listModelPrices.mockResolvedValue([]);
    render(
      <ReloadRegistryProvider value={register}>
        <Shelf onAdd={() => {}} />
      </ReloadRegistryProvider>,
    );
    await screen.findByText("DeepSeek");
    expect(apiMock.listCatalog).toHaveBeenCalledTimes(1);

    // The reader narrows the list, then asks for fresh data from the status bar.
    await user.click(screen.getByRole("button", { name: en.shelf.chipThird }));
    expect(screen.getByRole("button", { name: en.shelf.chipThird })).toHaveClass("active");
    expect(screen.queryByText("DeepSeek")).toBeNull();

    expect(reload, "the screen registered a reload").not.toBeNull();
    await act(() => reload!());

    // Read again…
    expect(apiMock.listCatalog).toHaveBeenCalledTimes(2);
    // …and the chip is still the one that was picked: the row it filters out is
    // still filtered out, which a remount would have undone.
    expect(screen.getByRole("button", { name: en.shelf.chipThird })).toHaveClass("active");
    expect(screen.queryByText("DeepSeek")).toBeNull();
    expect(screen.getByText("Kimi")).toBeInTheDocument();
  });
});

describe("a price with a schedule", () => {
  it("starts on the peak rates and switches to the off-peak ones in place", async () => {
    const user = userEvent.setup();
    const row = await renderShelf([entry("deepseek", "DeepSeek", { price_ref: PRICE })]);

    // The published row's own figures are the peak ones, so that is the cell.
    expect(within(row).getByText(cell("DeepSeek Chat", "9", "27"))).toBeInTheDocument();
    const chip = within(row).getByRole("button", { name: en.shelf.tierPricePeak });
    expect(chip).toHaveAttribute("aria-pressed", "false");

    await user.click(chip);
    expect(within(row).getByText(cell("DeepSeek Chat", "4.5", "13.5"))).toBeInTheDocument();
    expect(within(row).getByRole("button", { name: en.shelf.tierPriceOffPeak })).toHaveAttribute(
      "aria-pressed",
      "true",
    );

    // …and back: the switch is a peek at the other number, not a preference.
    await user.click(within(row).getByRole("button", { name: en.shelf.tierPriceOffPeak }));
    expect(within(row).getByText(cell("DeepSeek Chat", "9", "27"))).toBeInTheDocument();
  });

  it("spells out both rates and the window on the vendor's clock", async () => {
    const row = await renderShelf([entry("deepseek", "DeepSeek", { price_ref: PRICE })]);
    const chip = within(row).getByRole("button", { name: en.shelf.tierPricePeak });

    const title = chip.getAttribute("title") ?? "";
    // Both pairs, labelled — the two numbers are the same two numbers either way
    // round, so which is which is the whole content of the tooltip.
    expect(title).toContain(`${en.shelf.peakRates} ${rates("9", "27")}`);
    expect(title).toContain(`${en.shelf.offPeakRates} ${rates("4.5", "13.5")}`);
    // The window, named on the vendor's clock: the hours are Beijing's, and a
    // reader in another zone has to be told that rather than left to assume.
    expect(title).toContain(`${en.shelf.peakHours}: ${en.shelf.dayMon}/Tue/Wed/Thu/Fri 09:00–12:00`);
    expect(title).toContain(en.shelf.vendorTime.replace("{offset}", "+08:00"));
  });

  it("carries the step-up of a long request in the cell's own tooltip", async () => {
    const row = await renderShelf([entry("deepseek", "DeepSeek", { price_ref: PRICE })]);
    const priceCell = within(row).getByText(cell("DeepSeek Chat", "9", "27"));

    // The cell has room for the headline rate alone, and the headline is the band
    // most requests pay — so the second band is in the tooltip, where it is not
    // competing with the number being compared.
    // The sentence's own prefix ("above 512k tokens"), then the stepped-up pair:
    // the template carries both halves, so it is cut at the rates.
    const stepUp = en.shelf.priceLongContext.split("{rates}")[0].replace("{over}", "512k");
    const title = priceCell.getAttribute("title") ?? "";
    expect(title).toContain(stepUp);
    // The band's own pair, which is labelled like every other pair but carries no
    // schedule (`bandRatesText`'s join, not the cell's slash).
    expect(title).toContain(
      `${en.shelf.priceIn} ${fmtMoney(4.2, "CNY")} · ${en.shelf.priceOut} ${fmtMoney(16.8, "CNY")}`,
    );
  });
});

describe("a price with no variants", () => {
  it("offers nothing to switch and no step-up", async () => {
    const row = await renderShelf([
      entry("kimi", "Kimi", {
        price_ref: {
          model_id: "kimi-k2",
          display_name: "Kimi K2",
          input: "4",
          output: "16",
          currency: "CNY",
        },
      }),
    ]);

    expect(within(row).getByText(cell("Kimi K2", "4", "16"))).toBeInTheDocument();
    // A discount with no window is just a different price, and a single band has
    // nothing to switch to: the chip is absent rather than inert.
    expect(within(row).queryByRole("button", { name: en.shelf.tierPricePeak })).toBeNull();
    const flatTitle = within(row).getByText(cell("Kimi K2", "4", "16")).getAttribute("title") ?? "";
    expect(flatTitle).not.toContain("512k");
  });
});

describe("the detail dialog", () => {
  it("gives the step-up its own line for a model that publishes one", async () => {
    const user = userEvent.setup();
    const row = await renderShelf(
      [entry("deepseek", "DeepSeek", { price_ref: PRICE })],
      [
        modelPrice({
          long_context: {
            over: 512_000,
            in: "4.20",
            out: "16.80",
            cache_read: "0.84",
            cache_creation: "0",
          },
        }),
        modelPrice({ model_id: "deepseek-reasoner", display_name: "DeepSeek Reasoner" }),
      ],
    );

    await user.click(row);

    // The dialog reads the mirror's per-model rows, so a model priced in bands
    // there says so here — a band carried by only one of the two projections is
    // half the screens disagreeing.
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText(/above 512k tokens/)).toBeInTheDocument();
  });
});
