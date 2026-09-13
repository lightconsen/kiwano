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
};
