// What the dictionaries' types cannot see.
//
// A key the English side has and a translation does not is already a `tsc`
// failure inside `pnpm build` (see ../types.ts), so the key-set walk here is a
// re-check of a guarantee that mostly holds — worth having anyway, because it
// fails with the offending paths in the message instead of a type diff.
//
// The placeholder walk is the part nothing else covers. A message's values are
// plain strings to the type system, so `"{agent} is taken over"` translated
// without its `{agent}` is a perfectly valid `string`, and the screen renders a
// sentence with a hole in it. The runtime substitutes only what the call site
// passes, so nothing fails: the reader just sees the wrong text.
import { describe, expect, it } from "vitest";

import { en } from "./en";
import { DICTIONARIES, type Locale } from "./index";

type Leaves = Map<string, string>;

/** Every dotted leaf path of a dictionary and its string, e.g. `providers.inUse`. */
function leaves(dict: object, prefix = ""): Leaves {
  const out: Leaves = new Map();
  for (const [key, value] of Object.entries(dict)) {
    const path = prefix ? `${prefix}.${key}` : key;
    if (typeof value === "string") {
      out.set(path, value);
    } else if (value && typeof value === "object") {
      for (const [inner, text] of leaves(value, path)) out.set(inner, text);
    }
  }
  return out;
}

/** The `{name}` tokens a message substitutes, as a set — order is not the point. */
function placeholders(message: string): Set<string> {
  return new Set(message.match(/\{[a-zA-Z0-9_]+\}/g) ?? []);
}

/** 除英文外的每一门语言,连着它的 id —— 失败信息里要指出是哪一门。 */
const translated: [Locale, Leaves][] = (Object.keys(DICTIONARIES) as Locale[])
  .filter((l) => l !== "en")
  .map((l) => [l, leaves(DICTIONARIES[l])]);

const english = leaves(en);

/** Keep a failure's message readable: forty paths say less than the first ten. */
const head = (paths: string[]) => paths.slice(0, 10);

describe("the dictionaries", () => {
  it("hold the same keys", () => {
    const problems: string[] = [];
    for (const [locale, dict] of translated) {
      for (const k of english.keys()) if (!dict.has(k)) problems.push(`${locale} missing ${k}`);
      for (const k of dict.keys()) if (!english.has(k)) problems.push(`${locale} extra ${k}`);
    }
    expect(head(problems)).toEqual([]);
  });

  it("substitute the same placeholders", () => {
    const mismatched: string[] = [];
    for (const [locale, dict] of translated) {
      for (const [key, message] of english) {
        const text = dict.get(key);
        if (text === undefined) continue; // the key-set test reports it
        const want = placeholders(message);
        const got = placeholders(text);
        const same = want.size === got.size && [...want].every((p) => got.has(p));
        if (!same) {
          mismatched.push(
            `${locale} ${key}: ${[...want].join(",") || "none"} vs ${[...got].join(",") || "none"}`,
          );
        }
      }
    }
    expect(head(mismatched)).toEqual([]);
  });

  it("carry no empty message", () => {
    const empty: string[] = [];
    for (const [locale, dict] of [["en", english], ...translated] as [Locale, Leaves][]) {
      for (const [key, message] of dict) {
        if (message.trim() === "") empty.push(`${locale} ${key}: ${JSON.stringify(message)}`);
      }
    }
    expect(head(empty)).toEqual([]);
  });
});
