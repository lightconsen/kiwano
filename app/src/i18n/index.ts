// UI internationalisation — the runtime.
//
// A hand-written dictionary rather than a library. The repo has no i18n
// dependency and demonstrably avoids adding one (the landing page rolls its own,
// `sidecar.rs` writes HTTP by hand rather than pull in `reqwest`), but the
// decisive reason is safety: a library keyed by strings fails at runtime. These
// keys are typed against `en/`, so a missing or misspelled one is a `tsc`
// failure inside `pnpm build` — caught before anything renders, which a test
// could only tell you after it did. (`dictionaries.test.ts` covers the half
// types cannot: a translation that dropped a `{placeholder}`.)
//
// The locale is a module-level value with a listener set, mirroring
// `src/lib/updateInstall.ts` — the app already uses that shape for state that
// outlives a component, and the language must outlive every screen.
import { useEffect, useState } from "react";
import { en, type Messages } from "./en";
import { ja } from "./ja";
import { zhHans } from "./zh-Hans";
import { zhHant } from "./zh-Hant";
import type { KeyPath, Params } from "./types";

/** The locales we ship a dictionary for. This is the list everything else is
    derived from: the preference values, the dictionaries, the labels, and what
    `resolveLocale` will accept from the settings row or from a stored value. */
export const LOCALES = ["en", "zh-Hans", "zh-Hant", "ja"] as const;
export type Locale = (typeof LOCALES)[number];

/** What `AppSettings.language` holds: a preference, not necessarily a locale. */
export const LANGUAGE_PREFS = ["system", ...LOCALES] as const;
export type LanguagePref = (typeof LANGUAGE_PREFS)[number];

/** Every shipped locale's dictionary. Exported for the tests, which check each
    one against `en` for the keys and placeholders a translation can drop. */
export const DICTIONARIES: Record<Locale, Messages> = {
  en,
  "zh-Hans": zhHans,
  "zh-Hant": zhHant,
  ja,
};

/** What the language select shows. The label is a language's own name for
    itself, so it is not translated — a reader looking for their language
    should recognise it whatever the UI is currently in. */
export const LANGUAGE_LABELS: Record<LanguagePref, string> = {
  system: "", // filled by `languageLabel` below, which needs the current locale
  en: "English",
  "zh-Hans": "简体中文",
  "zh-Hant": "繁體中文",
  ja: "日本語",
};

/** Preferences written by an older build, or by hand, read back as today's id.
 *
 * 0.2.8 and earlier stored `"zh-CN"`; the id became `zh-Hans` when the UI
 * gained a Traditional Chinese dictionary, because `zh-CN` names a region
 * where script is what actually differs. Anything stored then has to keep
 * working — a preference is written once and read for the life of the
 * install, and there is no migration for a value the user chose — so the
 * region tags a person might have typed are accepted too. */
const LEGACY_PREFS: Record<string, Locale> = {
  "zh-CN": "zh-Hans",
  "zh-SG": "zh-Hans",
  "zh-TW": "zh-Hant",
  "zh-HK": "zh-Hant",
  "zh-MO": "zh-Hant",
  zh: "zh-Hans",
};

/** A BCP-47 tag from the webview → a locale we ship.
 *
 * Script before region: `zh-Hant`, `zh-TW`, `zh-HK` and `zh-MO` are all
 * Traditional, while `zh`, `zh-Hans`, `zh-CN` and `zh-SG` are Simplified —
 * which is why the ids name the script. A tag we do not ship falls through to
 * English rather than to the nearest relative: a half-understood language is
 * worse than a familiar one. */
function localeFromTag(tag: string): Locale {
  const t = tag.toLowerCase();
  if (t.startsWith("ja")) return "ja";
  if (t.startsWith("zh")) {
    return t.includes("hant") || /-(tw|hk|mo)\b/.test(t) ? "zh-Hant" : "zh-Hans";
  }
  return "en";
}

/** Turn a stored preference into a locale we have a dictionary for.
 *
 * `"system"` — and anything unrecognised, including the empty string a fresh
 * install may carry — resolves from the webview's `navigator.language`. That
 * reflects the OS setting on every platform Tauri targets, so this needs no
 * extra plugin. */
export function resolveLocale(pref: string | null | undefined): Locale {
  if (pref) {
    // The shipped ids first: this is what the settings row writes back, and a
    // value we ship must resolve to itself whatever it is and whoever is
    // browsing — the fallback below is for other people's tags, not ours.
    if ((LOCALES as readonly string[]).includes(pref)) return pref as Locale;
    if (pref in LEGACY_PREFS) return LEGACY_PREFS[pref];
  }
  const tag =
    typeof navigator !== "undefined" && navigator.language ? navigator.language : "en";
  return localeFromTag(tag);
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
