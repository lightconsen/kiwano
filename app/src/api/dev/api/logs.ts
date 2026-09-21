// The request log surface: the paged list, one row, the export and the clear.
// Its fixtures are in `../logs.ts`.

import type { KiwanoApi, RequestLogDetail, RequestLogExport, RequestLogFilter, RequestLogList } from "../../types";
import { delay } from "../delay";
import { matchingLogs, requestLogs } from "../logs";

export const logsApi: Pick<
  KiwanoApi,
  | "listRequestLogs"
  | "getRequestLog"
  | "exportRequestLogs"
  | "openLogFolder"
  | "clearRequestLogs"
> = {
  async listRequestLogs(page: number, pageSize: number, filter?: RequestLogFilter): Promise<RequestLogList> {
    await delay();
    const rows = matchingLogs(filter);
    const start = (page - 1) * pageSize;
    return { rows: rows.slice(start, start + pageSize).map((r) => ({ ...r })), total: rows.length };
  },

  async getRequestLog(id: number): Promise<RequestLogDetail | null> {
    await delay();
    return requestLogs.find((r) => r.id === id) ?? null;
  },

  // No filesystem in the browser, so this reports what it would have written
  // rather than pretending to write it — the toolbar's feedback then matches
  // the desktop app's without a second code path here. `_includeBodies` has
  // nothing to do without a file to shape, so it only has to be accepted.
  async exportRequestLogs(
    _path: string,
    filter?: RequestLogFilter,
    _includeBodies?: boolean,
  ): Promise<RequestLogExport> {
    await delay();
    return { rows_written: matchingLogs(filter).length, truncated: false };
  },

  async openLogFolder(): Promise<void> {
    // The browser mock has no filesystem to reveal.
    await delay();
  },

  async clearRequestLogs(): Promise<void> {
    await delay();
    requestLogs.length = 0;
  },
};
