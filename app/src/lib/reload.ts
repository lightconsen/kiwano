// Screen data reloads, registered while a screen is mounted.
//
// The app-level Refresh (the button in the status bar) used to remount the
// current screen by changing its `key`, which read its data again and everything
// else with it: the shelf's category chip and search box, the dashboard's window
// and filters, the scroll position, an open row's expanded state. What the button
// means is "read the data again", so each screen publishes how to do that and the
// button awaits it. The component stays mounted; only what came back changes.
//
// One screen is mounted at a time (the routes are exclusive), so the registry
// holds one reload. It is a ref rather than state on purpose: registering must
// not re-render the tree it is registered from.
import { createContext, useCallback, useContext, useEffect, useRef } from "react";

/** Re-read this screen's data. Resolves when it has landed. */
export type Reload = () => Promise<unknown>;

const ReloadContext = createContext<((fn: Reload) => () => void) | null>(null);

/** The registry itself, owned by the app shell. */
export function useReloadRegistry(): {
  register: (fn: Reload) => () => void;
  reload: () => Promise<unknown>;
} {
  const current = useRef<Reload | null>(null);
  const register = useCallback((fn: Reload) => {
    current.current = fn;
    // Unregister only if this is still the live one: a screen that remounts
    // (a route change) must not clear its successor's registration.
    return () => {
      if (current.current === fn) current.current = null;
    };
  }, []);
  const reload = useCallback(() => current.current?.() ?? Promise.resolve(), []);
  return { register, reload };
}

/** Hand the registry to the screens below (one provider, in the app shell). */
export const ReloadRegistryProvider = ReloadContext.Provider;

/** Publish this screen's reload for as long as it is mounted.
 *
 * `fn` may change identity between renders (it closes over filters, windows and
 * the like); the latest one is what the registry calls, so a refresh reads what
 * the screen is *showing* rather than what it showed when it mounted. */
export function useReload(fn: Reload): void {
  const register = useContext(ReloadContext);
  const latest = useRef(fn);
  latest.current = fn;
  useEffect(() => register?.(() => latest.current()), [register]);
}

