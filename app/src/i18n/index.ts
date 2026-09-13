// UI internationalisation — the runtime.
//
// A hand-written dictionary rather than a library. The repo has no i18n
// dependency and demonstrably avoids adding one (the landing page rolls its own,
// `sidecar.rs` writes HTTP by hand rather than pull in `reqwest`), but the
// decisive reason is safety: **this project has no frontend test runner**. A
// library keyed by strings fails at runtime, where nothing would catch it;
// these keys are typed against `en/`, so a missing or misspelled one is a `tsc`
// failure inside `pnpm build`, which is the only automated gate there is.
//
// The locale is a module-level value with a listener set, mirroring
// `src/lib/updateInstall.ts` — the app already uses that shape for state that
// outlives a component, and the language must outlive every screen.
import { useEffect, useState } from "react";
import { en, type Messages } from "./en";
import { zhCN } from "./zh-CN";
import type { KeyPath, Params } from "./types";

/** What `AppSettings.language` holds: a preference, not necessarily a locale. */
export const LANGUAGE_PREFS = ["system", "en", "zh-CN"] as const;
export type LanguagePref = (typeof LANGUAGE_PREFS)[number];

/** A locale we actually ship a dictionary for. */
export type Locale = Exclude<LanguagePref, "system">;

const DICTIONARIES: Record<Locale, Messages> = { en, "zh-CN": zhCN };

/** What the language select shows. The label is a language's own name for
    itself, so it is not translated — a reader looking for their language
    should recognise it whatever the UI is currently in. */
export const LANGUAGE_LABELS: Record<LanguagePref, string> = {
  system: "", // filled by `languageLabel` below, which needs the current locale
  en: "English",
  "zh-CN": "简体中文",
};

/** Turn a stored preference into a locale we have a dictionary for.
 *
 * `"system"` — and anything unrecognised, including the empty string a fresh
 * install may carry — resolves from the webview's `navigator.language`. That
 * reflects the OS setting on every platform Tauri targets, so this needs no
 * extra plugin. A preference naming a locale we do not ship falls back the same
 * way rather than to English, so a user whose OS is Chinese is not dropped into
 * English because of a typo in the settings row.
 */
export function resolveLocale(pref: string | null | undefined): Locale {
  if (pref === "en" || pref === "zh-CN") return pref;
  const tag =
    typeof navigator !== "undefined" && navigator.language ? navigator.language : "en";
  return tag.toLowerCase().startsWith("zh") ? "zh-CN" : "en";
}

let current: Locale = "en";
const listeners = new Set<() => void>();

/** Read a dotted path out of a dictionary, or `undefined` if it is not a leaf. */
function lookup(dict: Messages, path: string): string | undefined {
  let node: unknown = dict;
  for (const part of path.split(".")) {
    if (typeof node !== "object" || node === null) return undefined;
    node = (node as Record<string, unknown>)[part];
  }
  return typeof node === "string" ? node : undefined;
}

/** Substitute `{name}` placeholders. A missing parameter is left as written
    rather than becoming "undefined" — a visibly wrong message that still names
    the key is easier to act on than one that reads as a real sentence. */
function interpolate(message: string, params?: Params): string {
  if (!params) return message;
  return message.replace(/\{(\w+)\}/g, (whole, name: string) =>
    name in params ? String(params[name]) : whole,
  );
}

/** A bound translator. Returning this from a hook keeps components free of the
    locale, and means a re-render is the only thing a language change needs. */
export type Translate = (key: KeyPath<Messages>, params?: Params) => string;

export function translate(locale: Locale, key: KeyPath<Messages>, params?: Params): string {
  const message = lookup(DICTIONARIES[locale], key) ?? lookup(en, key);
  // Falling back to English rather than showing the raw key: a half-translated
  // screen is recoverable, a screen of dotted identifiers is not. The missing
  // key is still a `tsc` error, so this is a runtime safety net and not the
  // mechanism.
  return interpolate(message ?? key, params);
}

/** Set the active locale. Idempotent, and notifies subscribers only on change. */
export function setLocale(locale: Locale): void {
  if (locale === current) return;
  current = locale;
  for (const notify of listeners) notify();
}

export function getLocale(): Locale {
  return current;
}

/** The translator, and a subscription to language changes. */
export function useT(): Translate {
  const [, force] = useState(0);
  useEffect(() => {
    const notify = () => force((n) => n + 1);
    listeners.add(notify);
    return () => {
      listeners.delete(notify);
    };
  }, []);
  return (key, params) => translate(current, key, params);
}

/** The current locale, for the few places that need it rather than a message
    (e.g. `<html lang>`). */
export function useLocale(): Locale {
  const [, force] = useState(0);
  useEffect(() => {
    const notify = () => force((n) => n + 1);
    listeners.add(notify);
    return () => {
      listeners.delete(notify);
    };
  }, []);
  return current;
}

/** The label for the language select, which has to name `system` in whatever
    language the UI is currently in. */
export function languageLabel(t: Translate, pref: LanguagePref): string {
  if (pref === "system") return t("common.languageSystem");
  return LANGUAGE_LABELS[pref];
}

export type { Messages, KeyPath, Params };
