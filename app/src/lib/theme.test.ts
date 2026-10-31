// The theme's third choice: "system" resolves from the OS and keeps following
// it. What is pinned here is the split the whole feature rests on — the *choice*
// is stored and re-resolved, never the colour it resolved to at that moment —
// plus the one thing a transition-based `applyTheme` cannot cover on its own:
// the OS changing under a running window.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { applyTheme, resolveTheme, THEME_SYSTEM, watchSystemTheme } from "./theme";

/** A `matchMedia` stand-in: jsdom has none, and the tests need to drive it. */
function installMatchMedia(initialDark: boolean) {
  let dark = initialDark;
  const listeners = new Set<() => void>();
  const mq = {
    get matches() {
      return dark;
    },
    media: "(prefers-color-scheme: dark)",
    addEventListener: (_: string, cb: () => void) => listeners.add(cb),
    removeEventListener: (_: string, cb: () => void) => listeners.delete(cb),
  };
  window.matchMedia = vi.fn(() => mq) as unknown as typeof window.matchMedia;
  return {
    setDark(next: boolean) {
      dark = next;
      for (const cb of listeners) cb();
    },
  };
}

const isDark = () => document.documentElement.classList.contains("dark");

beforeEach(() => {
  localStorage.clear();
});

afterEach(() => {
  document.documentElement.classList.remove("dark");
  delete (window as { matchMedia?: unknown }).matchMedia;
});

describe("resolving a theme choice", () => {
  it("follows the OS exactly while the choice is system, and never for a fixed one", () => {
    const os = installMatchMedia(true);
    expect(resolveTheme(THEME_SYSTEM)).toBe("dark");
    os.setDark(false);
    expect(resolveTheme(THEME_SYSTEM)).toBe("light");
    // A fixed choice ignores the OS in both directions.
    expect(resolveTheme("dark")).toBe("dark");
    expect(resolveTheme("light")).toBe("light");
  });

  it("reads an unknown stored value as dark, which is how it read before", () => {
    installMatchMedia(false);
    // A downgrade or a hand-edited row: not a reason for the app to change
    // colour, and the pre-third-choice behaviour was "dark unless light".
    expect(resolveTheme("")).toBe("dark");
    expect(resolveTheme("midnight")).toBe("dark");
  });
});

describe("applying a theme choice", () => {
  it("stores the choice, not the colour it resolved to", () => {
    installMatchMedia(true);
    applyTheme(THEME_SYSTEM);
    expect(isDark()).toBe(true);
    // The point: the laptop being dark right now must not be written down, or
    // the app would stay dark after the OS goes light.
    expect(localStorage.getItem("kiwano.theme")).toBe(THEME_SYSTEM);
  });

  it("applies both fixed choices", () => {
    installMatchMedia(false);
    applyTheme("dark");
    expect(isDark()).toBe(true);
    applyTheme("light");
    expect(isDark()).toBe(false);
    expect(localStorage.getItem("kiwano.theme")).toBe("light");
  });
});

describe("following the OS while the window is open", () => {
  it("re-applies on an OS change, and only for the system choice", () => {
    const os = installMatchMedia(false);
    const stop = watchSystemTheme();

    applyTheme(THEME_SYSTEM);
    expect(isDark()).toBe(false);
    os.setDark(true);
    expect(isDark()).toBe(true);

    // A user who picked a colour is not moved by the OS changing under them.
    applyTheme("light");
    os.setDark(false);
    expect(isDark()).toBe(false);
    os.setDark(true);
    expect(isDark()).toBe(false);

    stop();
    os.setDark(false);
    expect(isDark()).toBe(false);
  });

  it("stops listening when unsubscribed", () => {
    const os = installMatchMedia(false);
    const stop = watchSystemTheme();
    applyTheme(THEME_SYSTEM);
    stop();
    os.setDark(true);
    // Nothing re-applied: still light, and still "system" in storage.
    expect(isDark()).toBe(false);
    expect(localStorage.getItem("kiwano.theme")).toBe(THEME_SYSTEM);
  });
});
