// What a hand-added provider declares it charges per million tokens.
import type { DeclaredPrice } from "../../api/types";
import type { KeyPath, Messages } from "../../i18n";

// This table carries user-visible labels, so it is built inside the form where
// the translator is available rather than at module scope (a module-level `t`
// is impossible — it is a hook).

/// The four rates one declared-price row collects, in the price table's own
/// order: `DeclaredPrice` is read back with exactly these fields, and the order
/// is the one every price in the app is printed in (input, output, cache read,
/// cache write).
export const RATE_FIELDS: {
  key: keyof Omit<DeclaredPrice, "model_id">;
  labelKey: KeyPath<Messages>;
}[] = [
  { key: "input", labelKey: "addProvider.priceIn" },
  { key: "output", labelKey: "addProvider.priceOut" },
  { key: "cache_read", labelKey: "addProvider.priceCacheRead" },
  { key: "cache_creation", labelKey: "addProvider.priceCacheWrite" },
];
