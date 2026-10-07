// Release notes in the reader's language.
//
// A release body is written once, in English, and published in three places
// that all read the same string: the GitHub release, the updater manifest, and
// the About row in Settings (`update.notes`). The app is shipped in four
// languages, so the About row was the one place a reader met English because of
// how the text travels rather than because of what it said.
//
// The fix is in how the body is written, not in how it is fetched: the English
// stays first, and a translation goes below a marker naming its locale.
//
//   ## [0.2.12] …
//   (English, the section the release workflow copies in)
//
//   <!-- notes:zh-Hans -->
//   (Chinese)
//
// A body with no marker for the reader's language is the whole body, which is
// what every release before this one looks like — so an untranslated release
// reads exactly as it did, and a translated one reads in the reader's language.
import type { Locale } from "../i18n";

/** `<!-- notes:zh-Hans -->`, tolerant of spacing inside the comment. */
function marker(locale: string): RegExp {
  return new RegExp(`<!--\\s*notes:${locale}\\s*-->`, "i");
}

const ANY_MARKER = /<!--\s*notes:([A-Za-z-]+)\s*-->/g;

/**
 * The part of `notes` written for `locale`.
 *
 * Falls back to the whole body — not to nothing — when there is no section for
 * this locale: an English-only release is information the reader can use, and
 * blanking it would be worse than the language they did not ask for.
 */
export function releaseNotesFor(notes: string, locale: Locale): string {
  const own = marker(locale);
  const hit = own.exec(notes);
  if (hit) {
    const rest = notes.slice(hit.index + hit[0].length);
    // Chained separators — an entry carrying three translations — each end at
    // the next marker rather than running on through it.
    const next = ANY_MARKER.exec(rest);
    ANY_MARKER.lastIndex = 0;
    return (next ? rest.slice(0, next.index) : rest).trim();
  }
  // No section for this locale. By the convention the English is what stands
  // before the first marker, so a body carrying translations still answers
  // `en` with the English rather than with all four languages at once. A body
  // that opens with a marker has no head to give, and falls through to the
  // whole thing below rather than to nothing.
  const first = ANY_MARKER.exec(notes);
  ANY_MARKER.lastIndex = 0;
  if (first) {
    const head = notes.slice(0, first.index).trim();
    if (head) return head;
  }
  return notes.trim();
}

/** Whether this body carries any translation at all — used by the tests, and
 *  by anyone asking why a release reads in English. */
export function hasTranslations(notes: string): boolean {
  const found = ANY_MARKER.test(notes);
  ANY_MARKER.lastIndex = 0;
  return found;
}
