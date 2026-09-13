// Keys for shelf. English is the source of truth for the key set;
// the zh-CN file must match this shape exactly (see ../types.ts).
//
// The four module-level label maps in Shelf.tsx (chips, billing, probe
// verdicts, columns) hold these keys rather than display text, because a
// module-level `const` cannot call the `useT` hook.
export const shelf = {
  chipAll: "All",
  chipOfficial: "Official",
  chipAggregate: "Aggregator",
  chipThird: "Third-party",
  chipFree: "Free tier",
  /** The `local` category badge: there is no chip for it, so it gets its own
      key rather than reusing a filter label that does not exist. */
  tagLocal: "Local",

  billingPlan: "Plan",
  billingPayg: "Pay as you go",
  billingUnl: "Unlimited",

  probeOk: "OK",
  probeAuth: "Auth required",
  probeUnsupported: "Unsupported",
  probeError: "Error",
  probeUnreachable: "Unreachable",
  /** Detail carried by the chip when the probe request itself failed. */
  probeFailed: "probe failed",

  colName: "Name",
  colProtocol: "Protocol",
  colCategory: "Category",
  colPrice: "Price",
  colActions: "Actions",

  test: "Test",
  endpoints: "ENDPOINTS",
  billing: "Billing",
  added: "Added",
  alreadyAdded: "Already added",
  connect: "+ Connect",
  add: "+ Add",

  fromHub: "From Kiwano Hub · {n} providers",
  searchPlaceholder: "Search providers…",
  refreshAria: "Refresh from Hub",
  refreshTitle: "Fetch the latest catalog from Kiwano Hub",
  noMatches: "No matching providers",
  /** Distinct from noMatches on purpose: an empty catalog is a different
   *  problem from a filter that matched nothing, and only one of them is the
   *  user's doing. There is no bundled fallback any more, so this is what a
   *  machine that has never reached the Hub sees. */
  notSynced: "No catalog yet — fetch it from Kiwano Hub",
};
