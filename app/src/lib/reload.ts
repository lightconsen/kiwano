// Screen data reloads, registered while a screen is mounted.
//
// The app-level Refresh (the button in the status bar) used to remount the
// current screen by changing its `key`, which read its data again and everything
// else with it: the shelf's category chip and search box, the dashboard's window
// and filters, the scroll position, an open row's expanded state. What the button
// means is "read the data again" — the data of the screen in front of you, not of
// anything outside it (settings are read once at launch, and agent detection
// spawns a login shell per agent, so neither belongs behind a click) — so each
// reader publishes how to do that and the button awaits it. The component stays
// mounted; only what came back changes.
//
// The exclusive routes mean one *screen* is mounted at a time, but a screen is
// not a single reader: the dashboard draws its cards and an embedded request-log
// table, and both have to re-read. So the registry holds a set, not one slot —
// with one slot the child registered first (React runs child effects before the
// parent's) and the parent's registration silently replaced it.
//
// It is a ref rather than state on purpose: registering must not re-render the
// tree it is registered from.
import { createContext, useCallback, useContext, useEffect, useRef } from "react";

/** Re-read this screen's data. Resolves when it has landed. */
export type Reload = () => Promise<unknown>;

const ReloadContext = createContext<((fn: Reload) => () => void) | null>(null);

/** The registry itself, owned by the app shell. */
export function useReloadRegistry(): {
  register: (fn: Reload) => () => void;
  reload: () => Promise<unknown>;
} {
  const current = useRef<Set<Reload>>(new Set());
  const register = useCallback((fn: Reload) => {
    current.current.add(fn);
    // The set is keyed by function identity, so an unregister can only ever
    // remove the one it added — a screen that remounts (a route change) cannot
    // clear a successor's registration, which is what the old single slot
    // needed an explicit `if (current === fn)` guard for.
    return () => {
      current.current.delete(fn);
    };
  }, []);
  // `Promise.resolve().then(fn)` rather than `fn()`: a reader that throws
  // synchronously (a bug in the reader, not a failed read) would otherwise come
  // out of the click handler synchronously and bypass the caller's catch.
  const reload = useCallback(
    () => Promise.all([...current.current].map((fn) => Promise.resolve().then(fn))),
    [],
  );
  return { register, reload };
}

/** Hand the registry to the screens below (one provider, in the app shell). */
export const ReloadRegistryProvider = ReloadContext.Provider;

/** Publish a reader's reload for as long as it is mounted.
 *
 * A screen registers for itself, and so does a panel embedded in one — both are
 * part of what that screen is showing.
 *
 * `fn` may change identity between renders (it closes over filters, windows and
 * the like); the latest one is what the registry calls, so a refresh reads what
 * the screen is *showing* rather than what it showed when it mounted. That
 * relies on `latest.current = fn` running during render: move it into the effect
 * and a refresh would re-read whatever page and filters the reader mounted with
 * instead of the ones on screen. */
export function useReload(fn: Reload): void {
  const register = useContext(ReloadContext);
  const latest = useRef(fn);
  latest.current = fn;
  useEffect(() => register?.(() => latest.current()), [register]);
}

