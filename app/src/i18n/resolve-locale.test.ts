// The preference → locale mapping, which is a migration as much as a lookup:
// `AppSettings.language` is written once and read for the life of an install,
// so a value stored by an older build has to keep working after the id it
// named is renamed.
import { afterEach, describe, expect, it } from "vitest";

import { resolveLocale } from "./index";

/** Pretend the webview reports `tag` as the system language. */
function withSystemLanguage(tag: string) {
  Object.defineProperty(navigator, "language", { value: tag, configurable: true });
}

const original = Object.getOwnPropertyDescriptor(navigator, "language");

afterEach(() => {
  if (original) Object.defineProperty(navigator, "language", original);
});

describe("resolveLocale", () => {
  it("reads back the ids an older build stored", () => {
    // 0.2.8 and earlier wrote "zh-CN"; the dictionary moved to zh-Hans when the
    // UI gained a Traditional one. No migration runs for this — the value is
    // simply read as what it means today.
    expect(resolveLocale("zh-CN")).toBe("zh-Hans");
    expect(resolveLocale("zh")).toBe("zh-Hans");
  });

  it("takes today's ids as they are", () => {
    expect(resolveLocale("en")).toBe("en");
    expect(resolveLocale("zh-Hans")).toBe("zh-Hans");
  });

  it("falls back to the system language when there is no preference", () => {
    withSystemLanguage("zh-CN");
    expect(resolveLocale("system")).toBe("zh-Hans");
    expect(resolveLocale(null)).toBe("zh-Hans");
    expect(resolveLocale(undefined)).toBe("zh-Hans");
    withSystemLanguage("zh-TW");
    expect(resolveLocale("system")).toBe("zh-Hans"); // until a Traditional dictionary ships
    withSystemLanguage("en-GB");
    expect(resolveLocale("system")).toBe("en");
    withSystemLanguage("de");
    expect(resolveLocale("system")).toBe("en");
  });

  it("sends an empty preference to the system language, not to English", () => {
    // A fresh install can carry an empty string. A Chinese user in that state
    // must not be dropped into English.
    withSystemLanguage("zh-Hans");
    expect(resolveLocale("")).toBe("zh-Hans");
  });
});
