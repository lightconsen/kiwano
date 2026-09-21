// The mock's key rows, kept raw the way the database keeps them and masked on
// the way out. Mirrors `crates/core/src/vm/keys.rs`.

import type { ApiKeyEntry } from "../types";

// Dev storage for rotating keys (spec §4.1 P1 multi-key rotation).
// Keeps the raw key the way the database does, and masks on the way out — the
// same contract the Rust side enforces, so the UI cannot come to depend on
// reading a real key here and break against the desktop build.
export type DevApiKeyRow = Omit<ApiKeyEntry, "masked"> & { provider_id: string; key: string };

export const devApiKeys: DevApiKeyRow[] = [];

/** Mirrors `mask_key` in crates/core/src/vm/keys.rs. */
function maskKey(key: string): string {
  const n = key.length;
  if (n > 12) return `${key.slice(0, 6)}…${key.slice(-4)}`;
  if (n > 4) return `…${key.slice(-4)}`;
  return "•".repeat(n);
}

export const toApiKeyEntry = ({ provider_id: _p, key, ...rest }: DevApiKeyRow): ApiKeyEntry => ({
  ...rest,
  masked: maskKey(key),
});
