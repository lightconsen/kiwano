// Theme selection: a class on <html>, the same one Tailwind's `dark:` variant
// is keyed on. The choice lives in settings (backend) so it survives a
// reinstall of the frontend, and is mirrored into localStorage so index.html
// can apply it before the first paint — otherwise the window flashes the
// default theme while settings load.
//
// Three values, not two: `"dark"`, `"light"`, and `"system"`, which resolves
// from the OS and keeps following it while the window is open. The *choice* is
// what gets stored, never the resolved colour — storing `"dark"` because the
// laptop was dark at the time would freeze the app there.
import { getCurrentWindow } from "@tauri-apps/api/window";

const STORAGE_KEY = "kiwano.theme";
const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

/** Follow the operating system's appearance. */
export const THEME_SYSTEM = "system";

const darkQuery = () =>
  typeof window !== "undefined" && typeof window.matchMedia === "function"
    ? window.matchMedia("(prefers-color-scheme: dark)")
    : null;

/** The colour a choice means right now: `"dark"` or `"light"`. */
export function resolveTheme(theme: string): "dark" | "light" {
  if (theme === THEME_SYSTEM) return darkQuery()?.matches ? "dark" : "light";
  // Anything that is not "light" is dark — the reading this had before there
  // was a third choice, kept so an unknown stored value (a downgrade, a
  // hand-edited row) cannot change how the app looks.
  return theme === "light" ? "light" : "dark";
}

/** The choice currently applied — what a live OS change should re-resolve. */
let current = "dark";

/** Apply a theme choice ("dark" | "light" | "system") to the document. */
export function applyTheme(theme: string): void {
  current = theme;
  const dark = resolveTheme(theme) === "dark";
  document.documentElement.classList.toggle("dark", dark);
  try {
    // The choice, not the colour: see the note at the top.
    localStorage.setItem(STORAGE_KEY, theme);
  } catch {
    // storage disabled: the setting still applies for this session
  }
  // The native window brings its own chrome (macOS draws the traffic lights
  // over an overlay title bar), and it follows the window's appearance, not
  // the document's — so it has to be told. `null` is the API's "follow the
  // system", which is exactly the third choice; a resolved colour sent here
  // would pin the chrome while the page followed the OS, and the two would
  // disagree the moment the OS changed.
  if (inTauri) {
    getCurrentWindow()
      .setTheme(theme === THEME_SYSTEM ? null : resolveTheme(theme))
      .catch(() => {});
  }
}

/**
 * Track the OS appearance and re-apply while the choice is `"system"`.
 *
 * A no-op for a fixed choice, so a user who picked dark is never moved by the
 * OS changing under them. Returns the unsubscribe, for a `useEffect` cleanup.
 *
 * This has to exist because `applyTheme` runs on a *transition*: with
 * `"system"`, flipping the OS setting produces no transition at all, and the
 * page would keep the class it resolved at mount.
 */
export function watchSystemTheme(): () => void {
  const mq = darkQuery();
  if (!mq) return () => {};
  const onChange = () => {
    if (current === THEME_SYSTEM) applyTheme(THEME_SYSTEM);
  };
  mq.addEventListener("change", onChange);
  return () => mq.removeEventListener("change", onChange);
}
