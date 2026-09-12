// Simplified Chinese dictionary. Shape-checked against `en/` by the `satisfies`
// below — a missing key is a `tsc` failure, not a silent English fallback.
import type { Shape } from "../types";
import type { Messages } from "../en";
import { common } from "./common";
import { app } from "./app";
import { settings } from "./settings";
import { providers } from "./providers";
import { addProvider } from "./addProvider";
import { logs } from "./logs";
import { dashboard } from "./dashboard";
import { shelf } from "./shelf";
import { strategy } from "./strategy";

export const zhCN = {
  common,
  app,
  settings,
  providers,
  addProvider,
  logs,
  dashboard,
  shelf,
  strategy,
} satisfies Shape<Messages>;
