// The multi-key surface: the rows a provider's keys are listed from, and the
// add and delete paths. Mirrors `crates/core/src/vm/keys.rs`.

import type { ApiKeyEntry, KiwanoApi, UpstreamKeySpendEntry } from "../../types";
import { nextId } from "../ids";
import { delay } from "../delay";
import { DevApiKeyRow, devApiKeys, toApiKeyEntry } from "../keys";

export const keysApi: Pick<
  KiwanoApi,
  | "listApiKeys"
  | "keyPoolSpend"
  | "addApiKey"
  | "deleteApiKey"
> = {
  async listApiKeys(providerId: string): Promise<ApiKeyEntry[]> {
    await delay();
    return devApiKeys.filter((k) => k.provider_id === providerId).map(toApiKeyEntry);
  },

  /** What each pool entry spent, derived from the provider's own usage fixture
      so the numbers on this screen come from the same table the Apps screen
      reads. A provider with no usage has no rows — which is the honest answer,
      not a zero. */
  async keyPoolSpend(providerId: string, _days: number): Promise<UpstreamKeySpendEntry[]> {
    await delay();
    const rows = devApiKeys.filter((k) => k.provider_id === providerId);
    if (rows.length === 0) return [];
    return rows.map((row, i) => {
      const requests = 120 + i * 45;
      return {
        upstream_key: row.label?.trim() || `#${row.id}`,
        currency: "USD",
        requests,
        tokens: requests * 9_400,
        unpriced_rows: 0,
        cost: requests * 0.036,
      };
    });
  },

  async addApiKey(providerId: string, apiKey: string, label?: string): Promise<ApiKeyEntry> {
    await delay();
    const row: DevApiKeyRow = {
      id: nextId(),
      provider_id: providerId,
      key: apiKey.trim(),
      label: label?.trim() || undefined,
      enabled: true,
      created_at: new Date().toISOString(),
    };
    devApiKeys.push(row);
    return toApiKeyEntry(row);
  },

  async deleteApiKey(id: number): Promise<void> {
    await delay();
    const i = devApiKeys.findIndex((k) => k.id === id);
    if (i >= 0) devApiKeys.splice(i, 1);
  },
};
