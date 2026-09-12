// The English dictionary — the source of truth for which keys exist.
//
// Every other locale is checked against this one's *shape* (see ../types.ts),
// so adding a key here without adding it there is a `tsc` failure rather than
// an English string surfacing in a translated UI.
import { common } from "./common";
import { app } from "./app";
import { settings } from "./settings";
import { providers } from "./providers";
import { addProvider } from "./addProvider";
import { logs } from "./logs";
import { dashboard } from "./dashboard";
import { shelf } from "./shelf";
import { strategy } from "./strategy";

export const en = {
  common,
  app,
  settings,
  providers,
  addProvider,
  logs,
  dashboard,
  shelf,
  strategy,
};

export type Messages = typeof en;
