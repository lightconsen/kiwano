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
  | "checkCredentialFinding"
  | "ackCredentialFinding"
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
  // the desktop app's without a second code path here.
  async exportRequestLogs(
    _path: string,
    filter?: RequestLogFilter,
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

  // The browser stand-in for the gateway's KV ack: session-scoped, which is
  // fine for a mock — a reload simply re-shows the banner.
  async checkCredentialFinding() {
    await delay();
    const latest = requestLogs.find((r) => r.request_notes?.startsWith("dlp:"));
    return latest && latest.id > ackedFindingId ? { ...latest } : null;
  },

  async ackCredentialFinding(id: number): Promise<void> {
    await delay();
    ackedFindingId = id;
  },
};

let ackedFindingId = 0;
