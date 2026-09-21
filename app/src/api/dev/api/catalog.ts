// The shelf's reads — the catalog, a model list per endpoint, the declared
// prices — and the probe and latency tests the add dialog runs.

import type { CatalogList, KiwanoApi, ModelPrice, ProbeReport, Protocol } from "../../types";
import { catalog, storedKeyFor } from "../catalog";
import { delay } from "../delay";
import { CATALOG_TOTAL, endpointKey } from "../endpoints";
import { providers } from "../providers";

export const catalogApi: Pick<
  KiwanoApi,
  | "testLatency"
  | "testEndpoint"
  | "listModels"
  | "listModelPrices"
  | "openUrl"
  | "listCatalog"
> = {
  async testLatency(endpoint: string): Promise<number> {
    await delay(400);
    const table: Record<string, number> = {
      "api.deepseek.com": 312,
      "api.moonshot.cn": 287,
      "open.bigmodel.cn": 305,
      "localhost:11434": 25,
    };
    return table[endpoint.replace(/^https?:\/\//, "")] ?? 260;
  },

  // Dev probe: pretend the protocol's models route answered (ok), unless the
  // endpoint is empty/decorated with "invalid" — exercises the chip styling
  async testEndpoint(
    _protocol: Protocol,
    endpoint: string,
    apiKey?: string,
    providerId?: string,
  ): Promise<ProbeReport> {
    await delay(500);
    const latency_ms = 200 + Math.floor(Math.random() * 300);
    if (!endpoint.trim() || endpoint.includes("invalid")) {
      return { verdict: "unreachable", status: null, latency_ms, detail: "connection failed (dev)" };
    }
    // Mirrors the backend: a blank key falls back to the stored one, but only
    // for an endpoint that provider already answers on.
    if (!apiKey?.trim() && !storedKeyFor(providerId, endpoint)) {
      return { verdict: "auth", status: 401, latency_ms, detail: "no key for this endpoint (dev)" };
    }
    return { verdict: "ok", status: 200, latency_ms, detail: "12 models listed (dev)" };
  },

  // Dev model list: resolves through the catalog entry matching the endpoint
  // (primary or per-protocol), falling back to a canned OpenAI-style list
  async listModels(
    _protocol: Protocol,
    endpoint: string,
    apiKey: string,
    providerId?: string,
  ): Promise<string[]> {
    await delay(600);
    if (!apiKey.trim() && !storedKeyFor(providerId, endpoint)) {
      throw new Error("no stored key covers this endpoint — enter one to fetch its models");
    }
    // Normalized, like the real matching: the form holds `api.moonshot.cn` where
    // the entry publishes `https://api.moonshot.cn`, and comparing the raw
    // strings missed every entry — so Fetch answered with the generic list even
    // for a provider the catalog knows.
    const ep = endpointKey(endpoint);
    const hit = catalog.find(
      (e) =>
        endpointKey(e.endpoint) === ep ||
        (e.endpoints ?? []).some((x) => endpointKey(x.endpoint) === ep),
    );
    if (hit) {
      return Array.from(new Set([...hit.models, ...(hit.endpoints ?? []).flatMap((x) => x.models ?? [])]));
    }
    return ["gpt-5.2", "gpt-5.2-mini", "o4-mini", "text-embedding-3-large"];
  },

  async listModelPrices(): Promise<ModelPrice[]> {
    await delay();
    // The mock's mirror: one row per catalog `price_ref`, plus the second model
    // DeepSeek prices by time of day — the case the dialog exists to show, and
    // one the catalog cannot (it names a single representative model).
    const rows: ModelPrice[] = catalog
      .filter((e) => e.price_ref)
      .map((e) => ({
        provider_id: e.id,
        model_id: e.price_ref!.model_id,
        display_name: e.price_ref!.display_name,
        input: e.price_ref!.input,
        output: e.price_ref!.output,
        cache_read: "0.03",
        cache_creation: "0",
        currency: e.price_ref!.currency,
        ...(e.price_ref!.off_peak ? { off_peak: e.price_ref!.off_peak } : {}),
        ...(e.price_ref!.peak_hours ? { peak_hours: e.price_ref!.peak_hours } : {}),
        ...(e.price_ref!.long_context ? { long_context: e.price_ref!.long_context } : {}),
      }));
    rows.push({
      provider_id: "deepseek",
      model_id: "deepseek-reasoner (R1)",
      display_name: "DeepSeek Reasoner (R1)",
      input: "0.55",
      output: "2.19",
      cache_read: "0.14",
      cache_creation: "0",
      currency: "USD",
      off_peak: { in: "0.28", out: "1.10", cache_read: "0.07", cache_creation: "0" },
      peak_hours: {
        tz_offset: 480,
        windows: [
          { days: ["mon", "tue", "wed", "thu", "fri"], start: "09:00", end: "12:00" },
          { days: ["mon", "tue", "wed", "thu", "fri"], start: "14:00", end: "18:00" },
        ],
      },
    });
    return rows;
  },

  async openUrl(url: string): Promise<void> {
    // Browser dev: no OS opener. Opening a tab is the closest thing, and the
    // real command is fenced to http(s) anyway.
    window.open(url, "_blank", "noopener");
  },

  async listCatalog(): Promise<CatalogList> {
    await delay();
    // Mirror the backend: `added` derives from the provider list at read
    // time, matching the primary endpoint OR any additional per-protocol
    // endpoint on either side (merged catalog entries stay one row)
    const keys = new Set(
      providers.flatMap((p) => [
        endpointKey(p.endpoint),
        ...(p.endpoints ?? []).map((e) => endpointKey(e.endpoint)),
      ]),
    );
    return {
      total: CATALOG_TOTAL,
      entries: catalog.map((e) => ({
        ...e,
        added:
          keys.has(endpointKey(e.endpoint)) ||
          (e.endpoints ?? []).some((x) => keys.has(endpointKey(x.endpoint))),
      })),
    };
  },
};
