// The release body's language split.
//
// The cases that matter are the two ends: a body written before this mechanism
// existed (no markers at all) must read exactly as it did, and a body carrying
// three languages must give each reader their own section and nothing else.
import { describe, expect, it } from "vitest";

import { hasTranslations, releaseNotesFor } from "./releaseNotes";

const EN = "### Added\n\n- The status bar says which version is running.";
const ZH_HANS = "### 新增\n\n- 状态栏会显示当前版本。";
const ZH_HANT = "### 新增\n\n- 狀態列會顯示目前版本。";
const JA = "### 追加\n\n- ステータスバーにバージョンを表示します。";

/** The shape the release workflow appends to a changelog section. */
const body = [EN, `<!-- notes:zh-Hans -->`, ZH_HANS, `<!-- notes:zh-Hant -->`, ZH_HANT, `<!-- notes:ja -->`, JA].join("\n\n");

describe("releaseNotesFor", () => {
  it("gives an untranslated body to everyone, whole", () => {
    // Every release before this mechanism looks like this, and a reader whose
    // language is not there is better served by English than by nothing.
    for (const locale of ["en", "zh-Hans", "zh-Hant", "ja"] as const) {
      expect(releaseNotesFor(EN, locale)).toBe(EN);
    }
  });

  it("gives each reader their own section, and only theirs", () => {
    expect(releaseNotesFor(body, "zh-Hans")).toBe(ZH_HANS);
    expect(releaseNotesFor(body, "zh-Hant")).toBe(ZH_HANT);
    expect(releaseNotesFor(body, "ja")).toBe(JA);
    // The English is what precedes the first marker.
    expect(releaseNotesFor(body, "en")).toBe(EN);
  });

  it("gives the English head to a language whose section is missing", () => {
    const twoLanguages = [EN, "<!-- notes:ja -->", JA].join("\n\n");
    expect(releaseNotesFor(twoLanguages, "ja")).toBe(JA);
    // A zh-Hans reader of that body gets the English, not the whole thing:
    // English is a language they may read, and the Japanese half is one they
    // may not. Only a body with no translations at all is handed over whole.
    expect(releaseNotesFor(twoLanguages, "zh-Hans")).toBe(EN);
    expect(releaseNotesFor(twoLanguages, "en")).toBe(EN);
  });

  it("stops at the next marker when a language has the last word", () => {
    // ja comes last, so slicing to the end is the same as slicing to a marker
    // that is not there — the case that would break a naive split.
    const tail = [EN, "<!-- notes:ja -->", JA].join("\n\n");
    expect(releaseNotesFor(tail, "ja")).not.toContain(EN);
  });

  it("accepts a marker however the workflow spaced it", () => {
    const loose = [EN, "<!--   notes:ja   -->", JA].join("\n\n");
    expect(releaseNotesFor(loose, "ja")).toBe(JA);
  });

  it("knows whether a body carries any translation", () => {
    expect(hasTranslations(EN)).toBe(false);
    expect(hasTranslations(body)).toBe(true);
    // Reused regexes carry lastIndex between calls; this is the check that
    // would go wrong first if that were forgotten.
    expect(hasTranslations(body)).toBe(true);
    expect(hasTranslations(EN)).toBe(false);
  });
});
