// The list's remembered preferences: the column sort and the view to open in.
// Both come from the settings blob, loaded once per screen; a save is
// fire-and-forget so a failed write never blocks the click that caused it.
import { useEffect, useState } from "react";
import { api } from "../../api/client";
import { formatSort, parseSort, type Sort } from "./order";
import { parseView, type ShelfView } from "./groups";

/** The list's remembered preferences: the column sort and the currency to price
    in. Both come from the settings blob, loaded once per screen; a save is
    fire-and-forget so a failed write never blocks the click that caused it. */
export function useShelfPrefs() {
  const [sort, setSort] = useState<Sort | null>(null);
  const [view, setView] = useState<ShelfView>("provider");
  useEffect(() => {
    let alive = true;
    api
      .getSettings()
      .then((s) => {
        if (!alive) return;
        setSort(parseSort(s.shelf_sort));
        setView(parseView(s.shelf_view));
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, []);
  const remember = (s: Sort | null) => {
    setSort(s);
    api.updateSettings({ shelf_sort: formatSort(s) }).catch(() => {});
  };
  // Persisted, not just held: App remounts the shelf after every mutation, so a
  // view in component state would snap back to the provider table the moment
  // the user added a provider while browsing models.
  const rememberView = (v: ShelfView) => {
    setView(v);
    api.updateSettings({ shelf_view: v }).catch(() => {});
  };
  return { sort, view, remember, rememberView };
}
