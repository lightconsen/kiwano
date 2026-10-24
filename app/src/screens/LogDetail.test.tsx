// One request log's own page: what a row click leaves to. The table's tests
// stop at "the row asks the Dashboard to navigate"; these pin what that page
// shows — the record, the two note blocks split apart, and the way back.
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { en } from "@/i18n/en";
import type { RequestLogDetail } from "@/api/types";

import LogDetail from "./LogDetail";

const { apiMock } = vi.hoisted(() => ({
  apiMock: { getRequestLog: vi.fn() },
}));

vi.mock("../api/client", () => ({ api: apiMock }));

function detail(over: Partial<RequestLogDetail> = {}): RequestLogDetail {
  return {
    id: 99,
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
    request_body: null,
    response_body: null,
    ...over,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  window.location.hash = "";
});

describe("a request log's page", () => {
  it("shows the record with its notes split, and the breadcrumb goes back", async () => {
    apiMock.getRequestLog.mockResolvedValue(
      detail({
        request_notes: "dlp: github-token ×1\nthinking: removed invalid value",
      }),
    );
    const user = userEvent.setup();
    render(<LogDetail logId={99} />);

    const title = en.logs.logTitle.replace("{id}", "99");
    expect(await screen.findByRole("heading", { name: title })).toBeInTheDocument();
    // The detector's finding is its own alert block; the shim's line stays
    // under the generic notes label.
    expect(screen.getByText(en.logs.detailCredentialWatch)).toBeInTheDocument();
    expect(screen.getByText(/dlp: github-token ×1/)).toBeInTheDocument();
    expect(screen.getByText(en.logs.detailNotes)).toBeInTheDocument();
    expect(screen.getByText(/thinking: removed invalid value/)).toBeInTheDocument();

    // The root is a control, and it says so on hover.
    const back = screen.getByRole("button", { name: en.app.nav.dashboard });
    expect(back.className).toContain("cursor-pointer");
    await user.click(back);
    expect(window.location.hash).toBe("#dashboard");

    // Read by id, not taken from a list that is no longer on screen.
    expect(apiMock.getRequestLog).toHaveBeenCalledWith(99);
  });

  it("reports a log the store no longer has, with the reason and a retry", async () => {
    apiMock.getRequestLog.mockRejectedValue(new Error("pruned"));
    render(<LogDetail logId={5} />);

    const alert = await screen.findByRole("alert");
    expect(alert.getAttribute("title")).toBe("pruned");
    expect(screen.getByRole("button", { name: en.common.retry })).toBeInTheDocument();
  });

  it("reads the id before anything else, and says it is loading meanwhile", () => {
    apiMock.getRequestLog.mockReturnValue(new Promise(() => {}));
    render(<LogDetail logId={7} />);
    expect(screen.getByText(en.common.loading)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: en.common.copy })).toBeDisabled();
  });
});
