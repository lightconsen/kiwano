// Browser-dev data source — numbers match the design/index.html prototype verbatim.
// During integration src/api/client.ts switches to the Tauri invoke implementation and this file is retired.

import type { KiwanoApi } from "./types";
import { providersApi } from "./dev/api/providers";
import { catalogApi } from "./dev/api/catalog";
import { dashboardApi } from "./dev/api/dashboard";
import { settingsApi } from "./dev/api/settings";
import { alertsApi } from "./dev/api/alerts";
import { keysApi } from "./dev/api/keys";
import { routesApi } from "./dev/api/routes";
import { appApi } from "./dev/api/app";
import { agentsApi } from "./dev/api/agents";
import { logsApi } from "./dev/api/logs";

export const devApi: KiwanoApi = {
  ...providersApi,
  ...catalogApi,
  ...dashboardApi,
  ...settingsApi,
  ...alertsApi,
  ...keysApi,
  ...routesApi,
  ...appApi,
  ...agentsApi,
  ...logsApi,
};
