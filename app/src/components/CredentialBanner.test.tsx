// The credential banner's three states: nothing unacked (no bar), a finding
// (bar with the detector's own note line), and either dismissal path — both
// ack the finding, and the click-through also deep-links the log's detail.
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { en } from "@/i18n/en";
import type { RequestLogEntry } from "@/api/types";

import { CredentialBanner } from "./CredentialBanner";

const { apiMock } = vi.hoisted(() => ({
  apiMock: {
    checkCredentialFinding: vi.fn(),
    ackCredentialFinding: vi.fn(),
  },
}));

vi.mock("../api/client", () => ({ api: apiMock }));

function finding(id: number): RequestLogEntry {
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
    request_notes: "dlp: github-token ×1",
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  apiMock.ackCredentialFinding.mockResolvedValue(undefined);
  window.location.hash = "";
});

describe("the credential banner", () => {
  it("stays hidden when there is nothing unacknowledged", async () => {
    apiMock.checkCredentialFinding.mockResolvedValue(null);
    const { container } = render(<CredentialBanner />);
    await waitFor(() => expect(apiMock.checkCredentialFinding).toHaveBeenCalled());
    expect(container).toBeEmptyDOMElement();
  });

  it("shows the finding's own note line, and the X acknowledges it", async () => {
    const user = userEvent.setup();
    apiMock.checkCredentialFinding.mockResolvedValue(finding(41));
    render(<CredentialBanner />);

    expect(await screen.findByText(en.app.credentialBannerTitle)).toBeInTheDocument();
    expect(screen.getByText("dlp: github-token ×1")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: en.app.credentialBannerDismiss }));
    expect(apiMock.ackCredentialFinding).toHaveBeenCalledWith(41);
    await waitFor(() =>
      expect(screen.queryByText(en.app.credentialBannerTitle)).toBeNull(),
    );
    // Dismissing goes nowhere.
    expect(window.location.hash).toBe("");
  });

  it("acknowledges and deep-links the log detail on click-through", async () => {
    const user = userEvent.setup();
    apiMock.checkCredentialFinding.mockResolvedValue(finding(41));
    render(<CredentialBanner />);

    await user.click(
      await screen.findByRole("button", { name: en.app.credentialBannerCta }),
    );
    expect(apiMock.ackCredentialFinding).toHaveBeenCalledWith(41);
    expect(window.location.hash).toBe("#dashboard/log/41");
    await waitFor(() =>
      expect(screen.queryByText(en.app.credentialBannerTitle)).toBeNull(),
    );
  });
});
