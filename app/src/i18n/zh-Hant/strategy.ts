export const strategy = {
  // ── Strategy labels ──
  single: "單一主用",
  failover: "容錯移轉",
  roundrobin: "加權輪詢",
  timewindow: "時間窗",
  quota: "配額遞補",
  leastBusy: "最少佔用",

  // ── One-line description shown beside the select ──
  singleHint: "一律使用主用 Provider",
  failoverHint: "故障時依序落到各備用，恢復後切回",
  roundrobinHint: "新工作階段依權重輪轉；同一工作階段維持黏著，以沿用上游提示快取",
  timewindowHint:
    "由候選的本機時間窗決定工作階段從何處開始；已在執行的工作階段維持原 Provider，沒有對應時間窗的請求則回退至主用",
  quotaHint: "當日主用達到下方數值後，之後開始的工作階段改發往備用 — 已在執行的工作階段維持原 Provider",
  leastBusyHint: "每個請求送往目前最閒置的健康候選；已在執行的工作階段維持原 Provider",

  // ── Quota-fallback unit select ──
  unitRequestsDay: "requests/天",
  unitTokensDay: "tokens/天",

  // ── Select trigger ──
  ariaFor: "{agent} 策略",

  // ── Copy-another-agent's-route row ──
  replaceConfirmOne: "用 {source} 的路由取代 {mine} 的（{strategy}，{count} 個候選）？",
  replaceConfirmOther: "用 {source} 的路由取代 {mine} 的（{strategy}，{count} 個候選）？",
  replace: "取代",
  copyRouteFrom: "複製路由自…",
  copyRouteAria: "將其他 Agent 的路由複製到 {agent}",
  pickAgent: "選擇 Agent",
  candidateOne: "{agent} · {count} 個候選",
  candidateOther: "{agent} · {count} 個候選",


  // ── The agent's own ceiling (its own row, under the strategy) ──
  limitAdd: "新增週期",
  limitRemoveAria: "移除此週期",
  limitLabel: "限額",
  limitNone: "不限",
  limitEditAria: "編輯 {agent} 的限額",
  limitTitle: "{agent} 的限額",
  limitPlaceholder: "不限",
  limitAmountAria: "限額數值",
  limitClear: "清除",
  limitUnitRequests: "次請求",
  limitUnitWanTokens: "萬 tokens",
  limitPeriodDay: "每天",
  limitPeriodWeek: "每週",
  limitPeriodMonth: "每月",
  limitPeriodYear: "每年",
  limitPeriodAll: "累計",
  limitBody:
    "統計此 Agent 在所有 Provider 上的用量，並在任何策略下生效 — 包括 single：它「僅使用一個 Provider」指的是不做容錯移轉，而不是可以不設限額。一個 Agent 可以同時設定多個週期，任何一個超限都會拒絕請求，直到該週期重置。",
  limitMoneyNote:
    "金額限額只統計價目表能定價的請求。模型沒有公開價格時，請求仍會計量但算不出金額，因此不計入限額。",
  limitCurrencyNote:
    "金額限額以此 Agent 的 Provider 計價貨幣為單位 — 其他貨幣的支出會先按 Hub 匯率換算後才計入。上限本身永不換算。",
};
