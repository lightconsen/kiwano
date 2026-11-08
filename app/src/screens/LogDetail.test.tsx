// One request log's own page: what a row click leaves to. The table's tests
// stop at "the row asks the Dashboard to navigate"; these pin what that page
// shows — the record, the two note blocks split apart, and the way back.
import { act, fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

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

/** jsdom ships no clipboard, and `copyText` (RequestLogs.tsx) tries the
    gesture-safe `execCommand` path first. These tests supply exactly that and
    leave the async fallback out of it. */
function grantClipboard(ok: boolean) {
  Object.defineProperty(document, "execCommand", {
    value: vi.fn(() => ok),
    configurable: true,
  });
  Object.defineProperty(navigator, "clipboard", { value: undefined, configurable: true });
}

afterEach(() => {
  Reflect.deleteProperty(document, "execCommand");
  Reflect.deleteProperty(navigator, "clipboard");
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

  it("caps a huge body at render, and only the whole thing on demand", async () => {
    // A body past the render cap is what used to hang the window: the webview
    // laid out every glyph of it. The detail now shows a bounded slice and
    // expands in place; nothing is fetched again.
    const big = "x".repeat(300 * 1024);
    apiMock.getRequestLog.mockResolvedValue(detail({ request_body: big }));
    const user = userEvent.setup();
    render(<LogDetail logId={11} />);

    await screen.findByText(en.logs.detailRequestBody);
    // The request-body pre is the one labelled by its heading, not the first
    // pre on the page — the headers render above it in the same grid.
    const heading = screen.getByText(en.logs.detailRequestBody);
    const block = heading.parentElement!;
    const pre = block.querySelector("pre")!;
    expect(pre.textContent!.length).toBeLessThan(big.length);
    expect(screen.getByText(new RegExp(en.logs.detailBodyCapped))).toBeInTheDocument();

    const expand = screen.getByRole("button", { name: en.logs.detailBodyExpand });
    await user.click(expand);
    expect(pre.textContent!.length).toBe(big.length);
    expect(screen.queryByRole("button", { name: en.logs.detailBodyExpand })).toBeNull();
  });
});

describe("the Copy button", () => {
  // `fireEvent`, not `userEvent`: user-event installs its own clipboard stub on
  // `navigator`, which `copyText` would then accept — the very path the webview
  // does not have, and the one these tests need to leave empty.
  it("says what it did, in words that outlast the gesture", async () => {
    grantClipboard(true);
    apiMock.getRequestLog.mockResolvedValue(detail());
    render(<LogDetail logId={99} />);
    const button = await screen.findByRole("button", { name: en.common.copy });

    // Fake timers before the click: the notice's timeout is armed by it, and a
    // timer armed on the real clock is not the one advanced below.
    vi.useFakeTimers();
    try {
      fireEvent.click(button);
      const status = screen.getByRole("status");
      expect(status).toHaveTextContent(en.logs.copiedToClipboard);

      // Copying again re-arms the four seconds: three seconds later it must
      // still be up, though the first click's deadline has passed.
      act(() => vi.advanceTimersByTime(3000));
      fireEvent.click(button);
      act(() => vi.advanceTimersByTime(3000));
      expect(status).toHaveTextContent(en.logs.copiedToClipboard);

      // Four seconds after the last click, the page stops claiming anything.
      act(() => vi.advanceTimersByTime(1000));
      expect(status).toHaveTextContent(/^$/);
    } finally {
      vi.useRealTimers();
    }
  });

  it("says so when the clipboard refused", async () => {
    grantClipboard(false);
    apiMock.getRequestLog.mockResolvedValue(detail());
    render(<LogDetail logId={99} />);

    fireEvent.click(await screen.findByRole("button", { name: en.common.copy }));

    expect(screen.getByRole("status")).toHaveTextContent(en.logs.copyFailed);
  });
});
