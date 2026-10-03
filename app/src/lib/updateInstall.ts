// Shared download-and-install state.
//
// The banner and Settings are two doors into the same transfer, and neither can
// own the state: the banner starts the download and then navigates, and Settings
// unmounts whenever you leave the page (App.tsx renders it per route). So the
// state sits here — the banner kicks it off, the route change mounts Settings,
// and Settings reads progress that is already running.
//
// One subscription to `update-progress` for the app's lifetime, established on
// the first start: the backend emits the event throughout the transfer and the
// operation outlives any component watching it.
import { useEffect, useState } from "react";
import { api } from "../api/client";

/** The three steps the backend reports. Everything before the first event is a
 *  `downloading` that has not produced a chunk yet. */
export type UpdatePhase = "downloading" | "installing" | "restarting";

export interface UpdateInstallState {
  installing: boolean;
  phase: UpdatePhase;
  /** Bytes so far, and the total from `Content-Length` when the server sent
   *  one. Null total means unknown — see `progress`. */
  downloaded: number;
  total: number | null;
  /**
   * Whole percent, or null when the response carried no Content-Length. The UI
   * reads null as "indeterminate" — it must not fall back to 0, which is what a
   * stalled transfer looks like.
   */
  progress: number | null;
  /** Bytes per second, averaged over the transfer so far. A 12 MB payload on a
   *  fast link is over in a second or two, and an average is the number that
   *  reads as movement when a percentage has no time to move. Null until there
   *  is enough of a sample to mean anything. */
  rate: number | null;
  /** Download failure. The version check keeps its own error — see Settings. */
  err: string | null;
}

let state: UpdateInstallState = {
  installing: false,
  phase: "downloading",
  downloaded: 0,
  total: null,
  progress: null,
  rate: null,
  err: null,
};
const listeners = new Set<() => void>();

function set(patch: Partial<UpdateInstallState>) {
  state = { ...state, ...patch };
  for (const notify of listeners) notify();
}

let watching = false;
let startedAt = 0;

/** Start or retry the download. Re-entrant calls are ignored, not queued. */
export function startUpdateInstall() {
  if (state.installing) return;

  if (!watching) {
    watching = true;
    // Subscribed before the first start so the Started event is not missed.
    api
      .onUpdateProgress((p) => {
        const downloaded = p.downloaded ?? 0;
        const total = p.total ?? null;
        const elapsed = (Date.now() - startedAt) / 1000;
        set({
          phase: p.phase ?? "downloading",
          downloaded,
          total,
          progress: total ? Math.round((downloaded / total) * 100) : null,
          // Averaged over the whole transfer rather than sampled between
          // events: a per-chunk rate jitters by an order of magnitude as
          // chunks arrive, which reads as noise rather than as speed. Nothing
          // until a fifth of a second in, so the first events do not print a
          // rate computed from a 5 ms sample.
          rate: elapsed > 0.2 && downloaded > 0 ? downloaded / elapsed : null,
        });
      })
      .catch(() => {});
  }

  startedAt = Date.now();
  set({
    installing: true,
    phase: "downloading",
    downloaded: 0,
    total: null,
    progress: 0,
    rate: null,
    err: null,
  });
  // Success relaunches the app, so only the failure path has state left to set.
  api.downloadAndInstallAppUpdate().catch((e) => {
    set({ err: String(e), installing: false });
  });
}

/** The state as it stands, for a caller that is not a component.
 *
 * `useLocale`/`getLocale` in `lib/i18n` are the same pair for the same reason:
 * module state with a subscription, plus a plain read for the places that only
 * need the value (here, the tests). */
export function getUpdateInstallState(): UpdateInstallState {
  return state;
}

export function useUpdateInstall(): UpdateInstallState {
  const [, force] = useState(0);

  useEffect(() => {
    const notify = () => force((n) => n + 1);
    listeners.add(notify);
    return () => {
      listeners.delete(notify);
    };
  }, []);

  return state;
}
