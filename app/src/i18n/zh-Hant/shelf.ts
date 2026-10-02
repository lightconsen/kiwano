export const shelf = {
  chipAll: "全部",
  chipOfficial: "官方",
  chipAggregate: "聚合",
  chipThird: "第三方",
  chipFree: "免費層",
  /** The `local` category badge: there is no chip for it, so it gets its own
      key rather than reusing a filter label that does not exist. */
  tagLocal: "本機",

  billingPlan: "方案",
  billingPayg: "按用量計費",
  billingUnl: "無限",
  /** Joined with `+`, deliberately unlike English: in the Chinese dictionary
   *  `+` is only a leading action prefix (`+ Add`, `+ Connect`), so between two
   *  words it reads as a plain list rather than "additionally"; `&` is a foreign
   *  sign here, where the dictionary writes 與 for a conjunction (`模型與價格`). */
  billingBoth: "PAYG + 方案",

  probeOk: "正常",
  probeAuth: "需要金鑰",
  probeUnsupported: "不支援",
  probeError: "錯誤",
  probeUnreachable: "無法連線",
  probeFailed: "探測失敗",

  colName: "名稱",
  colProtocol: "協定",
  colCategory: "分類",
  colBilling: "計費",
  colPrice: "價格",
  colActions: "操作",

  viewByProvider: "Provider",
  viewByModel: "模型",
  groupMeta: "{n} 個 Provider · {m} 個有價格",
  groupMetaOne: "1 個 Provider · {m} 個有價格",
  groupPriceTitle: "每百萬 token 的輸入價格，由低到高",

  tierPricePeak: "尖峰價",
  tierPriceOffPeak: "離峰價",
  tierPriceSwitch: "點擊切換尖峰與離峰價格",
  priceCacheRead: "命中",
  priceCacheWrite: "寫入",
  modelsPrices: "模型與價格 · {n} 個",
  modelsPricesFiltered: "模型與價格 · 符合 {n} / 共 {total} 個",
  searchModels: "搜尋模型…",
  noMatchingModels: "沒有符合的模型",
  showAllModels: "展開其餘 {n} 個",
  showFewerModels: "收合",
  priceLongContext: "超過 {over} tokens 後 · {rates}",
  noPublishedPrice: "未公布價格",
  website: "官網",
  priceIn: "入",
  priceOut: "出",
  priceUnit: "{rates}（每百萬 token）",
  peakRates: "尖峰",
  offPeakRates: "離峰",
  peakHours: "尖峰時段",
  vendorTime: "廠商時鐘 (UTC{offset})",
  windowSep: "、",
  dayMon: "週一",
  dayTue: "週二",
  dayWed: "週三",
  dayThu: "週四",
  dayFri: "週五",
  daySat: "週六",
  daySun: "週日",

  test: "測試",
  endpoints: "端點",
  billing: "計費",
  added: "已新增",
  alreadyAdded: "已新增過",
  connect: "+ 連線",
  add: "+ 新增",

  searchPlaceholder: "搜尋 Provider…",
  refreshAria: "從 Hub 重新整理",
  refreshTitle: "從 Kiwano Hub 取得最新目錄",
  noMatches: "沒有符合的 Provider",
  /** Distinct from noMatches on purpose: an empty catalog is a different
   *  problem from a filter that matched nothing, and only one of them is the
   *  user's doing. There is no bundled fallback any more, so this is what a
   *  machine that has never reached the Hub sees. */
  notSynced: "尚未取得目錄 —— 從 Kiwano Hub 取得",
};
