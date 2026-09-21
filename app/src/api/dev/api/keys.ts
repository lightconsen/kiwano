// The multi-key surface: the rows a provider's keys are listed from, and the
// add and delete paths. Mirrors `crates/core/src/vm/keys.rs`.

import type { ApiKeyEntry, KiwanoApi } from "../../types";
import { nextId } from "../ids";
import { delay } from "../delay";
import { DevApiKeyRow, devApiKeys, toApiKeyEntry } from "../keys";

export const keysApi: Pick<
  KiwanoApi,
  | "listApiKeys"
  | "addApiKey"
  | "deleteApiKey"
> = {
  async listApiKeys(providerId: string): Promise<ApiKeyEntry[]> {
    await delay();
    return devApiKeys.filter((k) => k.provider_id === providerId).map(toApiKeyEntry);
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
