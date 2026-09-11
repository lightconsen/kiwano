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

export interface UpdateInstallState {
  installing: boolean;
  /**
   * Whole percent, or null when the response carried no Content-Length. The UI
   * reads null as "indeterminate" — it must not fall back to 0, which is what a
   * stalled transfer looks like.
   */
  progress: number | null;
  /** Download failure. The version check keeps its own error — see Settings. */
  err: string | null;
}

let state: UpdateInstallState = { installing: false, progress: null, err: null };
const listeners = new Set<() => void>();

function set(patch: Partial<UpdateInstallState>) {
  state = { ...state, ...patch };
  for (const notify of listeners) notify();
}

let watching = false;

/** Start or retry the download. Re-entrant calls are ignored, not queued. */
export function startUpdateInstall() {
  if (state.installing) return;

  if (!watching) {
    watching = true;
    // Subscribed before the first start so the Started event is not missed.
    api
      .onUpdateProgress((p) => {
        set({ progress: p.total ? Math.round((p.downloaded / p.total) * 100) : null });
      })
      .catch(() => {});
  }

  set({ installing: true, progress: 0, err: null });
  // Success relaunches the app, so only the failure path has state left to set.
  api.downloadAndInstallAppUpdate().catch((e) => {
    set({ err: String(e), installing: false });
  });
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
