// The app's own surface — its config share, the footer's stats and its
// updater — which is everything here that is not about a provider, an agent, a
// log or a setting.

import type { ConfigShareReport, FooterStats, KiwanoApi, UpdateInfo, UpdateProgress } from "../../types";
import { delay } from "../delay";
import { providers } from "../providers";

export const appApi: Pick<
  KiwanoApi,
  | "exportConfig"
  | "importConfig"
  | "getFooterStats"
  | "checkAppUpdate"
  | "getPendingUpdate"
  | "downloadAndInstallAppUpdate"
  | "onUpdateProgress"
> = {
  async exportConfig(_path: string): Promise<number> {
    await delay(300);
    return providers.length;
  },

  async importConfig(_path: string): Promise<ConfigShareReport> {
    await delay(400);
    return { providers_added: 1, providers_kept: 2, routes_applied: 1 };
  },

  async getFooterStats(): Promise<FooterStats> {
    await delay();
    return {
      today_requests: 1284,
      today_tokens: 1_900_000,
      hub_synced: true,
      version: "v0.1.2",
    };
  },

  async checkAppUpdate(): Promise<UpdateInfo | null> {
    await delay();
    return null;
  },

  async getPendingUpdate(): Promise<UpdateInfo | null> {
    await delay();
    return null;
  },

  async downloadAndInstallAppUpdate(): Promise<void> {
    await delay();
  },

  async onUpdateProgress(_cb: (p: UpdateProgress) => void): Promise<() => void> {
    return () => {};
  },
};
