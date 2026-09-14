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
import { zhCN } from "./zh-CN";

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

const english = leaves(en);
const chinese = leaves(zhCN);

/** Keep a failure's message readable: forty paths say less than the first ten. */
const head = (paths: string[]) => paths.slice(0, 10);

describe("the dictionaries", () => {
  it("hold the same keys", () => {
    const missing = head([...english.keys()].filter((k) => !chinese.has(k)));
    const extra = head([...chinese.keys()].filter((k) => !english.has(k)));
    expect({ missing, extra }).toEqual({ missing: [], extra: [] });
  });

  it("substitute the same placeholders", () => {
    const mismatched: string[] = [];
    for (const [key, message] of english) {
      const translated = chinese.get(key);
      if (translated === undefined) continue; // the key-set test reports it
      const want = placeholders(message);
      const got = placeholders(translated);
      const same = want.size === got.size && [...want].every((p) => got.has(p));
      if (!same) {
        mismatched.push(`${key}: ${[...want].join(",") || "none"} vs ${[...got].join(",") || "none"}`);
      }
    }
    expect(head(mismatched)).toEqual([]);
  });

  it("carry no empty message", () => {
    const empty = head(
      [...english, ...chinese]
        .filter(([, message]) => message.trim() === "")
        .map(([key, message]) => `${key}: ${JSON.stringify(message)}`),
    );
    expect(empty).toEqual([]);
  });
});
