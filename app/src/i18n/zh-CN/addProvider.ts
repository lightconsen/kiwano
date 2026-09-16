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
  protoGemini: "Gemini 原生",

  apiKey: "API Key",
  keyKeep: "留空则保留当前密钥",
  keyLocal: "仅存本机，只有你能读取",
  showHideKey: "显示 / 隐藏密钥",

  endpointUrl: "端点 URL",
  endpointHintShelf: "来自模型库 · 每种协议一个 · 共用 API 密钥",
  addEndpoint: "添加端点",
  removeEndpoint: "移除此端点",
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
  billingBoth: "该厂商两种计费都有 —— 请为这条 Provider 选择一种",

  // ── Plan-mode percent limits ──
  usageLimits: "用量上限",
  usageLimitsHint: "可选 · 各套餐窗口的百分比",
  fiveHourWindow: "5 小时窗口",
  weeklyWindow: "每周窗口",
  planLimitsBody:
    "当该 Provider 某个窗口的实时套餐配额使用率达到该百分比时，Kiwano 会停止向其路由；用量回落到阈值以下后自动恢复。留空表示该窗口不设上限。",
  planLimitRange: "填 1–100，留空表示不限制",

  // ── Payg spending limit ──
  spendingLimit: "消费上限",
  spendingLimitHint: "可选 · 留空则显示用量趋势",
  currencyTitle: "{currency} —— 该 Provider 的计费货币",

  // ── 自填价格 ──
  // 仅在按量付费、且表单归用户自己所有时出现：模型库只为自己条目的模型定价，
  // 而一个不属于任何条目的 Provider 没有可用的公开价格。当条目接管表单（从模型库
  // 添加）时该区块隐藏，与协议、端点、货币三者的规则一致。
  prices: "价格",
  pricesHint: "可选 · 每百万 token，单位 {currency}",
  priceModel: "模型",
  priceModelPlaceholder: "模型 ID",
  priceIn: "输入",
  priceOut: "输出",
  priceCacheRead: "缓存读取",
  priceCacheWrite: "缓存写入",
  addPrice: "添加模型",
  removePrice: "移除该模型的价格",
  pricesBody:
    "该 Provider 的收费标准。这些数字用于计算它的请求成本，消费上限也以此为基准，因此优先于模型库的价格。价格留空的模型按模型库的表计算，查不到则记为未定价；缓存价格留空表示该项不计费。",
  priceInvalid: "每项价格都必须是 0 或以上的数字",

  // ── Unlimited ──
  noQuotaEndpoint: "厂商未提供额度查询接口 · 无法仅凭你的 Key 读取套餐用量",
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

  // 套餐查询不再在这里选：它来自 catalog 条目，而条目只会发「仅凭 API Key 即可查询」的模板。
  // 模板标签保留，因为那张表仍用于把已存的凭据字段原样带回去。

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
