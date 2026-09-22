// strategy — Simplified Chinese. The key set is not decided here: `en/` is
// the source of truth and this file must match its shape (see ../types.ts).
//
// Chinese has no plural inflection, so the one/other candidate-count keys hold
// the same value. That is correct, not an oversight.
export const strategy = {
  // ── Strategy labels ──
  single: "单主用",
  failover: "故障转移",
  roundrobin: "加权轮询",
  timewindow: "时间窗口",
  quota: "配额兜底",
  leastBusy: "最少占用",

  // ── One-line description shown beside the select ──
  singleHint: "始终使用主用 Provider",
  failoverHint: "故障时按顺序落到各备用，恢复后切回",
  roundrobinHint: "新会话按权重轮转；同一会话保持粘性，以复用上游提示缓存",
  timewindowHint:
    "由候选的本地时间窗口决定会话从哪开始；已在跑的会话保持原 provider，没有窗口匹配时回退到主用",
  quotaHint: "当天主用达到下方数值后，新会话改发往备用；已在跑的会话保持原 provider",
  leastBusyHint: "每个请求发往当前最空闲的健康候选；已在跑的会话保持原 provider",

  // ── Quota-fallback unit select ──
  unitRequestsDay: "requests/天",
  unitTokensDay: "tokens/天",

  // ── Select trigger ──
  ariaFor: "{agent} 策略",

  // ── Copy-another-agent's-route row ──
  replaceConfirmOne: "用 {source} 的路由替换 {mine} 的（{strategy}，{count} 个候选）？",
  replaceConfirmOther: "用 {source} 的路由替换 {mine} 的（{strategy}，{count} 个候选）？",
  replace: "替换",
  copyRouteFrom: "复制路由自…",
  copyRouteAria: "从其他 Agent 复制路由到 {agent}",
  pickAgent: "选择 Agent",
  candidateOne: "{agent} · {count} 个候选",
  candidateOther: "{agent} · {count} 个候选",


  // ── Agent 自身的限额（独立一行，位于策略下方）──
  limitAdd: "添加周期",
  limitRemoveAria: "移除此周期",
  limitLabel: "限额",
  limitNone: "不限",
  limitEditAria: "编辑 {agent} 的限额",
  limitTitle: "{agent} 的限额",
  limitPlaceholder: "不限",
  limitAmountAria: "限额数值",
  limitClear: "清除",
  limitUnitRequests: "次请求",
  limitUnitWanTokens: "万 tokens",
  limitPeriodDay: "每天",
  limitPeriodWeek: "每周",
  limitPeriodMonth: "每月",
  limitPeriodYear: "每年",
  limitPeriodAll: "累计",
  limitBody:
    "统计该 Agent 在所有 Provider 上的用量，并在任何策略下生效 —— 包括 single：它的「只有主选」说的是不做故障转移，不是可以不限额。一个 Agent 可以同时设多个周期，其中任何一个超限都会拒绝请求，直到该周期重置。",
  limitMoneyNote:
    "金额限额只统计价目表能定价的请求。模型没有公开价格时，请求仍会计量但算不出金额，因此不计入额度。",
  limitCurrencyNote:
    "金额限额用该 Agent 的 Provider 计价的货币书写 —— 其它货币的支出会按 Hub 汇率折算后再计入。上限本身永不换算。",
};
