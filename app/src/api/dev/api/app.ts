// The app's own surface — its config share, the footer's stats and its
// updater — which is everything here that is not about a provider, an agent, a
// log or a setting.

import type { ConfigShareReport, FooterStats, KiwanoApi, UpdateInfo, UpdateProgress } from "../../types";
import { delay } from "../delay";
// app/package.json is the version source of truth (`scripts/release.sh` bumps it).
import pkg from "../../../../package.json";
import { providers } from "../providers";

/** The footer's counters, held as one mutable object rather than rebuilt per
    call: the site's demo moves them (`src/demo/simulate.ts`) so the status bar
    reads like a machine that is doing something, and a fresh literal each time
    would drop every edit on the floor. */
export const footerStats: FooterStats = {
  today_requests: 1284,
  today_tokens: 1_900_000,
  hub_synced: true,
  // From app/package.json, which is the file `scripts/release.sh` bumps — so the
  // number a visitor reads in the demo's status bar is the version being
  // released. Hand-writing it here would have it drift one release behind the
  // hero's, which now also names a version.
  version: `v${pkg.version}`,
};

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
    return { ...footerStats };
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
