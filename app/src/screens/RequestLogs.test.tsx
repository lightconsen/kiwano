// The request-log table's own behaviours: what the pager asks for, what a
// status tab does to the page, and the three ways the export can end.
//
// It is embedded in the Dashboard too (`<RequestLogs agent providerId />`), so
// the filter narrowing is covered there; these are the parts that only exist
// when the table is used for itself. The export is the one that leaves the
// machine, which is why all three outcomes are pinned — including the two that
// write nothing.
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { en } from "@/i18n/en";
import type { RequestLogEntry } from "@/api/types";

import RequestLogs from "./RequestLogs";

const { apiMock, saveMock } = vi.hoisted(() => ({
  apiMock: {
    listRequestLogs: vi.fn(),
    getRequestLog: vi.fn(),
    exportRequestLogs: vi.fn(),
  },
  saveMock: vi.fn(),
}));

vi.mock("../api/client", () => ({ api: apiMock }));
// The file dialog is a Tauri plugin, called straight from the screen; cancelling
// it is `resolve(null)`, which is a decision and not a failure.
vi.mock("@tauri-apps/plugin-dialog", () => ({ save: saveMock }));

/** ONE_SHAPE: a log row with nothing exotic about it. */
function entry(id: number): RequestLogEntry {
  return {
    id,
    ts: "2026-09-14T10:00:00Z",
    method: "POST",
    path: "/v1/messages",
    query: null,
    agent: "claude",
    attribution: "key",
    provider_id: "deepseek",
    model: "deepseek-chat",
    status_code: 200,
    error_kind: null,
    error_message: null,
    session_id: null,
    is_streaming: false,
    input_tokens: 1_000,
    output_tokens: 200,
    cache_read_tokens: 0,
    cache_creation_tokens: 0,
    reasoning_tokens: 0,
    usage_missing: false,
    latency_ms: 900,
    first_token_ms: 300,
    request_headers: null,
    response_headers: null,
    request_size: 0,
    response_size: 0,
    truncated: false,
    request_notes: null,
  };
}

/** A page of rows, `total` in the database. */
function page(ids: number[], total: number) {
  return { rows: ids.map(entry), total };
}

async function openExportDialog() {
  const user = userEvent.setup();
  render(<RequestLogs />);
  // The toolbar's button is disabled until the first page has answered, and a
  // click on a disabled button is swallowed — so wait for it to be enabled.
  await waitFor(() =>
    expect(screen.getByRole("button", { name: en.logs.exportCsv })).toBeEnabled(),
  );
  await user.click(screen.getByRole("button", { name: en.logs.exportCsv }));
  return { user, dialog: await screen.findByRole("dialog") };
}

beforeEach(() => {
  vi.clearAllMocks();
  apiMock.listRequestLogs.mockResolvedValue(page([1, 2, 3], 25));
  saveMock.mockResolvedValue("/tmp/kiwano-logs.csv");
  apiMock.exportRequestLogs.mockResolvedValue({ rows_written: 3, truncated: false });
});

describe("the request log table", () => {
  it("pages through the query and returns to the first page when the slice changes", async () => {
    const user = userEvent.setup();
    render(<RequestLogs />);
    await waitFor(() => expect(apiMock.listRequestLogs).toHaveBeenCalledWith(1, 10, {}));

    // 25 rows at 10 a page: three pages, and Next is the way through them.
    expect(screen.getByText(en.logs.pageIndicator.replace("{page}", "1").replace("{pages}", "3"))).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: en.logs.next }));
    await waitFor(() => expect(apiMock.listRequestLogs).toHaveBeenLastCalledWith(2, 10, {}));

    // A status tab changes *which rows there are*, not which page of them is on
    // screen — so the page goes back to the start rather than asking for page 2
    // of a set nobody has seen page 1 of.
    await user.click(screen.getByRole("button", { name: en.logs.filterError }));
    await waitFor(() =>
      expect(apiMock.listRequestLogs).toHaveBeenLastCalledWith(1, 10, { status: "error" }),
    );
  });

  it("writes nothing when the file dialog is cancelled", async () => {
    saveMock.mockResolvedValue(null);
    const { user, dialog } = await openExportDialog();

    await user.click(within(dialog).getByRole("button", { name: en.logs.exportCsv }));

    await waitFor(() => expect(saveMock).toHaveBeenCalled());
    expect(apiMock.exportRequestLogs).not.toHaveBeenCalled();
    // No notice and no error: cancelling is not an outcome worth reporting.
    expect(within(dialog).queryByText(/Exported/)).toBeNull();
  });

  it("exports the chosen path", async () => {
    const { user, dialog } = await openExportDialog();

    // There is no bodies switch to flip: the file always carries them, so the
    // call is the path and the filter and nothing else.
    await user.click(within(dialog).getByRole("button", { name: en.logs.exportCsv }));

    await waitFor(() =>
      expect(apiMock.exportRequestLogs).toHaveBeenCalledWith(
        "/tmp/kiwano-logs.csv",
        expect.any(Object),
      ),
    );
    // The dialog closes on success, and the row count is reported where the
    // toolbar can show it.
    expect(await screen.findByText(en.logs.exported.replace("{n}", "3"))).toBeInTheDocument();
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("keeps the dialog open and says so when the export is capped", async () => {
    apiMock.exportRequestLogs.mockResolvedValue({ rows_written: 500_000, truncated: true });
    const { user, dialog } = await openExportDialog();

    await user.click(within(dialog).getByRole("button", { name: en.logs.exportCsv }));

    // A capped file is a partial answer: the reader has to narrow the range and
    // try again, so the dialog stays put with the reason in it. (The toolbar
    // repeats the line — `err` is also rendered there — so this is scoped.)
    expect(
      await within(dialog).findByText(
        en.logs.exportCapped.replace("{n}", (500_000).toLocaleString()),
      ),
    ).toBeInTheDocument();
    expect(screen.getByRole("dialog")).toBeInTheDocument();
  });
});
