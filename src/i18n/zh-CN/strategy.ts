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

  // ── One-line description shown beside the select ──
  singleHint: "始终使用主用 Provider",
  failoverHint: "故障时按顺序落到各备用，恢复后切回",
  roundrobinHint: "新会话按权重轮转；同一会话保持粘性，以复用上游提示缓存",
  timewindowHint: "按各候选的本地时间窗口选择；没有窗口匹配时回退到主用",
  quotaHint: "当天主用达到下方数值后，改发往备用",

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
};
