// Theme selection: a class on <html>, the same one Tailwind's `dark:` variant
// is keyed on. The choice lives in settings (backend) so it survives a
// reinstall of the frontend, and is mirrored into localStorage so index.html
// can apply it before the first paint — otherwise the window flashes the
// default theme while settings load.
import { getCurrentWindow } from "@tauri-apps/api/window";

const STORAGE_KEY = "kiwano.theme";
const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

/** Apply a theme name ("dark" | anything else → light) to the document. */
export function applyTheme(theme: string): void {
  const dark = theme !== "light";
  document.documentElement.classList.toggle("dark", dark);
  try {
    localStorage.setItem(STORAGE_KEY, dark ? "dark" : "light");
  } catch {
    // storage disabled: the setting still applies for this session
  }
  // The native window brings its own chrome (macOS draws the traffic lights
  // over an overlay title bar), and it follows the window's appearance, not
  // the document's — so it has to be told.
  if (inTauri) {
    getCurrentWindow()
      .setTheme(dark ? "dark" : "light")
      .catch(() => {});
  }
}
