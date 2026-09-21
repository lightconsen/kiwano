// The endpoint vocabulary the provider form and the catalog probe share: how a
// protocol is spelled in a row's note, and what counts as the same endpoint.
// Mirrors `vm::endpoint_note` and `vm::endpoint_key`.

import type { NewProviderInput, Protocol } from "../types";

export function protocolNote(protocol: NewProviderInput["protocol"]): string {
  return protocol === "openai" ? "OpenAI-compatible" : "Anthropic";
}

// endpoint_note suffix mirrors vm::endpoint_note ("OpenAI-compatible · +Anthropic")
export function endpointNote(
  protocol: NewProviderInput["protocol"],
  endpoints?: { protocol: Protocol }[],
): string {
  const tags = (endpoints ?? []).map((e) =>
    `+${e.protocol === "openai" ? "OpenAI" : "Anthropic"}`,
  );
  return [protocolNote(protocol), ...tags].join(" · ");
}

// Mirrors the merged bundled catalog (protocol siblings folded in; runapi.co
// was dead and dropped in favor of the live runapi.host)
export const CATALOG_TOTAL = 82;

// Endpoint identity: host+path, lowercased, scheme and trailing slashes
// stripped — mirrors vm::endpoint_key on the backend
export function endpointKey(s: string): string {
  return s
    .trim()
    .toLowerCase()
    .replace(/^https?:\/\//, "")
    .replace(/\/+$/, "");
}
