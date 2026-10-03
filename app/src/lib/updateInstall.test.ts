// The transfer's derived state: what the UI is handed while an update runs.
//
// Worth testing at this level because the interesting parts are all derived —
// the percentage from bytes and a total that may be missing, the rate from a
// clock — and because the last three phases (download, install, restart) are
// indistinguishable from a hang if any of them reports the wrong thing.
import { beforeAll, describe, expect, it, vi } from "vitest";

import { getUpdateInstallState, startUpdateInstall } from "./updateInstall";

const { apiMock, progress } = vi.hoisted(() => ({
  apiMock: {
    onUpdateProgress: vi.fn(),
    downloadAndInstallAppUpdate: vi.fn(),
  },
  progress: { cb: null as null | ((p: unknown) => void) },
}));

vi.mock("../api/client", () => ({ api: apiMock }));


describe("the update transfer's state", () => {
  // The subscription is module-level and lasts the app's lifetime — the lib
  // subscribes once and never again, which is the point of it. So the callback
  // is captured once here and driven by each test in turn, rather than being
  // re-registered per test the way a component-level effect would be.
  beforeAll(async () => {
    apiMock.onUpdateProgress.mockImplementation(async (cb: (p: unknown) => void) => {
      progress.cb = cb;
      return () => {};
    });
    // Never resolves: the transfer is what these tests are inside of.
    apiMock.downloadAndInstallAppUpdate.mockReturnValue(new Promise(() => {}));
    startUpdateInstall();
    await vi.waitFor(() => expect(progress.cb).not.toBeNull());
  });

  it("reports bytes, percent and rate while downloading", () => {
    progress.cb!({ phase: "downloading", downloaded: 6_150_000, total: 12_300_000 });
    expect(getUpdateInstallState().phase).toBe("downloading");
    expect(getUpdateInstallState().downloaded).toBe(6_150_000);
    expect(getUpdateInstallState().progress).toBe(50);
  });

  it("has no percent when the server sent no length, and does not pretend to have one", () => {
    progress.cb!({ phase: "downloading", downloaded: 3_000_000, total: null });
    expect(getUpdateInstallState().progress).toBeNull(); // the UI must not draw a 0% bar for this
    expect(getUpdateInstallState().downloaded).toBe(3_000_000);
  });

  it("carries the later phases through, where there is no length at all", () => {
    progress.cb!({ phase: "installing", downloaded: 12_300_000, total: 12_300_000 });
    expect(getUpdateInstallState().phase).toBe("installing");
    progress.cb!({ phase: "restarting", downloaded: 12_300_000, total: 12_300_000 });
    expect(getUpdateInstallState().phase).toBe("restarting");
  });

  it("treats an event with no phase as a download", () => {
    // Older binaries do not send `phase`; the frontend and the Rust side ship
    // together, but a dev-server frontend can talk to an older one.
    progress.cb!({ downloaded: 1_000_000, total: 12_300_000 });
    expect(getUpdateInstallState().phase).toBe("downloading");
  });
});
