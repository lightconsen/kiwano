// addProvider — Simplified Chinese. The key set is not decided here: `en/` is
// the source of truth and this file must match its shape (see ../types.ts).
export const addProvider = {
  // ── Dialog chrome ──
  titleAdd: "添加 Provider",
  titleEdit: "编辑 Provider",

  // ── Mode switch ──
  fromModels: "从模型库",
  fromModelsNamed: "从模型库：{name}",
  custom: "自定义",

  // ── Catalog picker ──
  searchCatalog: "搜索模型库…",
  noMatching: "没有匹配的 Provider",
  change: "更换",

  // ── Form fields ──
  name: "名称",
  protocol: "协议",
  protoOpenai: "兼容 OpenAI",
  protoAnthropic: "Anthropic",
  protoGemini: "Gemini API",

  apiKey: "API Key",
  keyKeep: "留空则保留当前密钥",
  keyLocal: "仅保存在本机钥匙串",
  showHideKey: "显示 / 隐藏密钥",

  endpointUrl: "端点 URL",
  endpointHintShelf: "来自模型库 · 每种协议一个 · 共用 API 密钥",
  endpointHint: "每种协议一个 · 共用 API 密钥",
  test: "测试",

  // ── Inline probe verdicts (full detail lives in the tooltip) ──
  probeOk: "OK",
  probeAuth: "鉴权",
  probeUnsupported: "404",
  probeError: "错误",
  probeUnreachable: "不可达",

  // ── Default model ──
  defaultModel: "默认模型",
  fetch: "获取",
  fetching: "获取中…",
  errorNoKey: "请先填写 API Key —— 提供商会拒绝匿名的模型列表请求",
  errorNoModels: "该端点未返回任何模型",
  modelIdPlaceholder: "模型 ID",

  // ── Billing ──
  billing: "计费",
  billingPlan: "套餐",
  billingPayg: "按量付费",
  billingUnlimited: "不限量",
  billingFixed: "该 Provider 固定",
  billingFromCatalog: "来自模型库",
  billingUnknown: "无法识别的计费标签“{billing}” · 请选择一种模式",

  // ── Plan-mode percent limits ──
  usageLimits: "用量上限",
  usageLimitsHint: "可选 · 各套餐窗口的百分比",
  planFiveHourPlaceholder: "例如 20 · 留空表示不限制",
  planWeeklyPlaceholder: "例如 60 · 留空表示不限制",
  fiveHourWindow: "5 小时窗口",
  weeklyWindow: "每周窗口",
  planLimitsBody:
    "当该 Provider 某个窗口的实时套餐配额使用率达到该百分比时，Kiwano 会停止向其路由；用量回落到阈值以下后自动恢复。配置套餐查询后生效。",

  // ── Payg spending limit ──
  spendingLimit: "消费上限",
  spendingLimitHint: "可选 · 留空则显示用量趋势",
  currencyTitle: "{currency} —— 该 Provider 的计费货币",

  // ── Unlimited ──
  noQuotaConfig: "未配置配额 · 列表中不显示用量信息",

  // ── Agent binding ──
  bindAgents: "保存后绑定到 Agent",
  selectAgents: "选择 Agent…",

  // ── Rotating keys (edit mode) ──
  rotatingKeys: "轮换密钥",
  rotatingKeysHint: "与主密钥轮换使用 · 规避速率限制",
  deleteKey: "删除密钥",
  addKeyPlaceholder: "sk-… 添加密钥",
  labelPlaceholder: "标签",

  // ── Plan quota query (edit mode) ──
  planQuotaQuery: "套餐配额查询",
  template: "模板",
  none: "无",
  planQueryBody: "使用该 Provider 的 API Key 查询套餐用量（5 分钟缓存）；配额标签会在 Provider 页面刷新。",

  // ── Advanced forwarding settings ──
  advanced: "高级（超时 / 重试 / 请求头）",
  timeoutLabel: "超时（秒）",
  retriesLabel: "重试次数",
  advancedBody:
    "留空使用网关默认值 · 超时仅限制等待响应头的时间，不会中断进行中的流 · 重试在故障转移前对该 Provider 生效",
  customHeaders: "自定义请求头",
  customHeadersHint: "最后合并 · 可覆盖 API Key 请求头",
  headerNamePlaceholder: "Header-Name",
  headerValuePlaceholder: "值",
  removeHeader: "移除请求头",
  addHeader: "添加请求头",

  // ── Footer ──
  saving: "保存中…",
  saveEnable: "保存并启用",

  // ── Plan-query templates ──
  // Product names are not translated; only the qualifier around them is.
  tplKimi: "Kimi（月度套餐）",
  tplZhipuPersonal: "Zhipu GLM（个人）",
  tplZhipuTeam: "Zhipu GLM（团队）",
  tplMinimax: "MiniMax",
  tplZenmux: "ZenMux",
  tplOpencodeGo: "OpenCode Go",
  tplVolcengine: "Volcengine Ark",
  fieldOrgId: "组织 ID",
  fieldProjectId: "项目 ID",
  fieldQuotaUrl: "用量接口地址",
  fieldAccessKeyId: "AccessKey ID",
  fieldSecretAccessKey: "Secret AccessKey",
};
