// providers — Simplified Chinese. The key set is not decided here: `en/` is
// the source of truth and this file must match its shape (see ../types.ts).
export const providers = {
  // ── Segment bar and list header ──
  all: "全部",
  counts: "{providers} 个 Provider · 已绑定 {agents} 个 Agent",
  addProvider: "添加 Provider",
  refreshTitle: "刷新 Provider 与套餐配额（绕过 5 分钟配额缓存）",

  // ── Per-agent onboarding (empty tab) ──
  takenOverTitle: "{agent} 已接管",
  takenOverBody: "本地网关正在转发该 Agent 的请求，但尚未绑定任何 Provider —— 请添加一个，让请求有处可去。",
  startManaging: "开始用 Kiwano 管理 {agent}",
  startManagingBody:
    "Kiwano 会备份当前配置（之后可一键还原），导入 {agent} 已在使用的 Provider（与其他 Agent 共享），并通过本地网关转发 —— 上游不变，之后可即时切换。",
  enableKiwano: "启用 Kiwano",
  enabling: "启用中…",
  addProviderFirst: "先添加 Provider",
  oauthNote:
    "使用官方订阅登录（Claude / Gemini 登录、Codex ChatGPT）？官方 OAuth 目前无法代理 —— 请先手动添加一个 Provider。",

  // ── Provider rows ──
  unbound: "未绑定",
  inUse: "使用中",
  editProvider: "编辑 Provider",
  deleteProvider: "删除 Provider",
  clickAgain: "再次点击以确认",
  confirm: "确认",
  blocked: "已阻断",
  healthy: "正常 {latency}ms",
  notRoutingTitle: "未在此处路由：{reason}",

  // ── Usage / quota cell ──
  usagePlanLimits: "套餐 · 上限 {windows} · 以该 Provider 的套餐配额使用率为准{tokens}",
  usageWindowFive: "5 小时窗口 ≤ {percent}%",
  usageWindowWeekly: "每周窗口 ≤ {percent}%",
  usageInOut: " · 输入 {input} · 输出 {output}",
  usagePlanQuota: "套餐 · 本周期 {used}/{limit} {unit}（{percent}%）· 重置于 {resets} · 输入 {input} · 输出 {output}",
  usageLocalInference: "本地推理 · 不计费 · 可离线使用",
  usagePaygQuota:
    "按量付费 · 本周期 {used} / {limit}（{percent}%）· 输入 {input}（缓存 {cache}，按 1/10 计费）· 输出 {output} · 延迟 {latency}",
  usagePaygTrend: "按量付费 · 未设置上限 · 近 7 天用量趋势",
  planTemplateCached: "{template} · 已缓存",
  planLimitFive: "5h ≤ {percent}%",
  planLimitWeekly: "周 ≤ {percent}%",
  unitReq: "次",
  unitTenKTok: "万 tok",
  resetsAt: "重置于 {date}",
  reqTokSuffix: "次 · {tokens} tok",
  inOutTokens: "输入 {input} · 输出 {output}",
  limitSuffix: "/ 上限 {limit}",
  tokensLatency: "{tokens} tokens · 延迟 {latency}",
  reqSuffix: "· {requests} 次",

  // ── Column headers ──
  colProvider: "Provider",
  colRole: "策略中的角色",
  colBoundAgents: "绑定的 Agent",
  colUsage: "用量 / 配额",
  colStatus: "状态",
  colPriority: "优先级",
  colActions: "操作",

  // ── Agent-tab role cell ──
  weight: "权重",
  weightTitle: "会话按权重比例在各候选之间轮转",
  windowStart: "窗口开始",
  windowEnd: "窗口结束",
  windowTitle:
    "该候选生效的本地时间窗口；直接输入数字，如 0930 —— 结束时间必须晚于开始时间。两者都清空即可移除窗口",
  fallback: "兜底",
  fallbackTitle: "当所有候选的窗口都不匹配时使用",
  noWindow: "未设窗口",
  noWindowTitle: "未设置窗口 —— 该候选永远不会被选中",
  primary: "主用",
  primaryTitle: "该 Agent 当前的路由",
  standby: "备用 #{index}",

  // ── Candidate row actions ──
  moveUp: "上移",
  moveDown: "下移",
  makePrimary: "设为主用",
  makePrimaryTitle: "设为主用 —— 将该候选移到队列首位",
  removeFromRouteAria: "从路由中移除",
  removeFromRoute: "从该路由中移除（其他 Agent 的绑定不受影响）",

  // ── Bind-another-provider row ──
  bindAria: "将 Provider 绑定到路由",
  allBound: "所有 Provider 均已绑定",
  bindExisting: "绑定已有 Provider…",
  allInRoute: "所有 Provider 都已在当前路由中 —— 请从顶部添加新的",
  bindAnother: "再绑定一个 Provider 到此路由 —— 它会作为备用加入队尾",

  // ── Empty list ──
  none: "还没有 Provider ——",

  // ── Closing note (one variant per tab state) ──
  footerNotTakenOver:
    "Kiwano 尚未路由该 Agent —— 请在上方启用接管；已保存的路由会保留并原样再次生效 · API 密钥仅存本机，只有你能读取 · 请求不经过 Kiwano 云端",
  footerStrategy:
    "Agent 路由策略 · 上方各行即按优先级排序的候选（主用在前）· 切换即时生效 · API 密钥仅存本机，只有你能读取 · 请求不经过 Kiwano 云端",
  footerDefault:
    "切换即时生效（该 Agent 已由本地网关接管；切换仅改变路由）· API 密钥仅存本机，只有你能读取 · 请求不经过 Kiwano 云端",

  // ── Plan quota tiers ──
  tierFiveHour: "5 小时窗口",
  tierWeekly: "每周",
  tierMonthly: "每月",
};
