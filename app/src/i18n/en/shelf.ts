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
  /** A vendor that charges both ways at one address (Anthropic: an API and a
      subscription).
   *
   * "PAYG" is this app's own word for pay-as-you-go — the billing chips already
   read `PAYG / Plan / Unl` — and it is the only form short enough to name two
   modes in a 104px column: "Pay as you go + plan" measures 113px against the
   88px that fits.
   *
   * The conjunction is `&`, which is how this dictionary joins two things a
   label offers (`Save & enable`, `MODELS & PRICES`, `Download & install`). `+`
   means *additional* here — the endpoint note reads `+Anthropic`, the actions
   read `+ Add` — and "PAYG + Plan" would borrow that sense to say the two
   charges stack, when the vendor offers either. */
  billingBoth: "PAYG & Plan",

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
  colBilling: "Billing",
  colPrice: "Price",
  colActions: "Actions",

  /** The segment group that switches how the table is grouped: two readings of
      one catalog, one row per provider or one group per model with its
      providers nested. The labels are short because the control itself says
      "group by" — both readings sit side by side. */
  viewByProvider: "Providers",
  viewByModel: "Models",
  /** A model group's header: how many providers serve it, and how many of them
      publish a price for it (the rest show a dash). */
  groupMeta: "{n} providers · {m} with a price",
  groupMetaOne: "1 provider · {m} with a price",
  /** Tooltip for the header's price range — a bare range carries no unit. */
  groupPriceTitle: "Input price per million tokens, lowest to highest",

  /** Time-of-day pricing: the chip in a price cell whose number depends on when
      the request runs, and the two labels the detail dialog's price block uses. */
  /** The two figures in a price cell are per-million-token rates, and are
      meaningless unlabelled — `¥9 / ¥27` could be any pair of numbers. */
  /** The switch in a tiered price cell: it reads as the tier being shown, and
      clicking it shows the other one. */
  tierPricePeak: "Peak price",
  tierPriceOffPeak: "Off-peak price",
  tierPriceSwitch: "Click to switch between the peak and off-peak prices",
  /** The four rates a price row can carry, and the section that lists them. */
  priceCacheRead: "cache hits",
  priceCacheWrite: "cache write",
  modelsPrices: "MODELS & PRICES · {n}",
  /** The same heading under a filter: what is left, out of what there was. */
  modelsPricesFiltered: "MODELS & PRICES · {n} of {total}",
  searchModels: "Search models…",
  noMatchingModels: "No model matches",
  showAllModels: "Show the other {n}",
  showFewerModels: "Show fewer",
  /** A model priced in length bands: the rates above the listed one apply once a
      request's input passes the threshold. The listed rate is the lower band, so
      this is a step *up* — and a request that ignores it is billed low. */
  priceLongContext: "above {over} tokens · {rates}",
  noPublishedPrice: "No published price",
  website: "Website",
  priceIn: "in",
  priceOut: "out",
  /** The cell's tooltip: the same rates, with the unit they are quoted in. */
  priceUnit: "{rates} per million tokens",
  peakRates: "Peak",
  offPeakRates: "Off-peak",
  peakHours: "Peak hours",
  /** Names the clock a schedule is written in. The windows are the vendor's
      business hours, so they are never converted into the reader's zone. */
  vendorTime: "Vendor's clock (UTC{offset})",
  windowSep: ", ",
  dayMon: "Mon",
  dayTue: "Tue",
  dayWed: "Wed",
  dayThu: "Thu",
  dayFri: "Fri",
  daySat: "Sat",
  daySun: "Sun",

  test: "Test",
  endpoints: "ENDPOINTS",
  billing: "Billing",
  added: "Added",
  alreadyAdded: "Already added",
  connect: "+ Connect",
  add: "+ Add",

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
