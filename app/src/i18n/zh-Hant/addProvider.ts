export const addProvider = {
  // ── Dialog chrome ──
  titleAdd: "新增 Provider",
  titleEdit: "編輯 Provider",

  // ── Mode switch ──
  fromModels: "從模型庫",
  fromModelsNamed: "從模型庫：{name}",
  custom: "自訂",

  // ── Catalog picker ──
  searchCatalog: "搜尋模型庫…",
  noMatching: "沒有符合的 Provider",
  change: "更換",

  // ── Form fields ──
  name: "名稱",
  protocol: "協定",
  protoOpenai: "相容 OpenAI",
  protoAnthropic: "Anthropic",
  protoGemini: "Gemini 原生",

  apiKey: "API Key",
  keyKeep: "留空則保留目前的金鑰",
  keyLocal: "僅儲存在本機，只有你能讀取",
  showHideKey: "顯示 / 隱藏金鑰",

  endpointUrl: "端點 URL",
  endpointHintShelf: "來自模型庫 · 每種協定一個 · 共用 API 金鑰",
  addEndpoint: "新增端點",
  removeEndpoint: "移除此端點",
  endpointHint: "每種協定一個 · 共用 API 金鑰",
  test: "測試",

  // ── Inline probe verdicts (full detail lives in the tooltip) ──
  probeOk: "OK",
  probeAuth: "驗證",
  probeUnsupported: "404",
  probeError: "錯誤",
  probeUnreachable: "無法連線",

  // ── Default model ──
  defaultModel: "預設模型",
  fetch: "取得",
  fetching: "取得中…",
  errorNoKey: "請先填寫 API Key —— 供應商會拒絕匿名的模型清單請求",
  errorNoModels: "該端點未回傳任何模型",
  errorTemplate: "請先補齊端點參數 —— URL 中還有未填的佔位符",
  modelIdPlaceholder: "模型 ID",

  // ── Billing ──
  billing: "計費",
  billingPlan: "方案",
  billingPayg: "按用量計費",
  billingUnlimited: "無限",
  billingFixed: "此 Provider 固定",
  billingFromCatalog: "來自模型庫",
  billingUnknown: "無法辨識的計費標籤「{billing}」 · 請選擇一種模式",
  billingBoth: "此供應商兩種計費方式都有 —— 請為這個 Provider 選擇一種",

  // ── Plan-mode percent limits ──
  usageLimits: "用量上限",
  usageLimitsHint: "選填 · 各方案窗口的百分比",
  fiveHourWindow: "5 小時窗口",
  weeklyWindow: "每週窗口",
  planLimitsBody:
    "當此 Provider 某個窗口的即時方案配額使用率達到該百分比時，Kiwano 會停止將請求路由至它；用量回落到該數值以下後自動恢復。留空表示該窗口不設上限。",
  planLimitRange: "填 1–100，留空表示不限",

  // ── Payg spending limit ──
  spendingLimit: "消費上限",
  spendingLimitHint: "選填 · 留空則顯示用量趨勢",
  currencyTitle: "{currency} —— 此 Provider 的計費貨幣",

  // ── Declared prices ──
  // Offered on pay-as-you-go, for a provider the form is the user's own: the
  // Hub prices the models of its catalog entries, and a provider that names none
  // has no published rate to be costed at. The section hides while an entry owns
  // the form (adding from Models), exactly as the protocol, the endpoints and the
  // currency do.
  prices: "價格",
  pricesHint: "選填 · 每百萬 token，單位 {currency}",
  priceModel: "模型",
  priceModelPlaceholder: "模型 ID",
  priceIn: "輸入",
  priceOut: "輸出",
  priceCacheRead: "快取讀取",
  priceCacheWrite: "快取寫入",
  addPrice: "新增模型",
  removePrice: "移除此模型的價格",
  pricesBody:
    "此 Provider 的收費標準。這些數字用於計算它的請求成本，消費上限也以此為基準，因此優先於模型庫的價格。價格留空的模型會依模型庫的價目表計算，查不到則記為未定價；快取價格留空表示該項不計費。",
  priceInvalid: "每項價格都必須是 0 或以上的數字",

  // ── Unlimited ──
  noQuotaEndpoint: "廠商未提供配額查詢端點 · 無法只憑你的 Key 讀取方案用量",
  noQuotaConfig: "未設定配額 · 清單中不顯示用量資訊",

  // ── Agent binding ──
  bindAgents: "儲存後綁定至 Agent",
  selectAgents: "選擇 Agent…",

  // ── Rotating keys (edit mode) ──
  rotatingKeys: "輪換金鑰",
  rotatingKeysHint: "與主金鑰輪換使用 · 避開速率限制",
  deleteKey: "刪除金鑰",
  addKeyPlaceholder: "sk-… 新增金鑰",
  labelPlaceholder: "標籤",

  // The plan-quota query is no longer chosen here: it comes from the catalog
  // entry, which only ever publishes a template that needs nothing but the
  // provider's own API key. The template labels below stay because the table
  // still maps a stored config's credential fields when one is re-saved.

  // ── Advanced forwarding settings ──
  advanced: "進階（逾時 / 重試 / 標頭）",
  timeoutLabel: "逾時（秒）",
  retriesLabel: "重試次數",
  advancedBody:
    "留空使用閘道預設值 · 逾時僅限制等待回應標頭的時間，不會中斷進行中的串流 · 重試在容錯移轉前對此 Provider 生效",
  customHeaders: "自訂標頭",
  customHeadersHint: "最後合併 · 可覆寫 API Key 標頭",
  customHeadersAuthWarning:
    "{name} 會覆寫閘道為此 Provider 注入的憑證。僅在該端點確實需要時保留。",
  headerNamePlaceholder: "Header-Name",
  headerValuePlaceholder: "值",
  removeHeader: "移除標頭",
  addHeader: "新增標頭",

  // ── Footer ──
  saving: "儲存中…",
  saveEnable: "儲存並啟用",

  // ── Plan-query templates ──
  // Product names are not translated; only the qualifier around them is.
  tplKimi: "Kimi（月費方案）",
  tplZhipuPersonal: "Zhipu GLM（個人）",
  tplZhipuTeam: "Zhipu GLM（團隊）",
  tplMinimax: "MiniMax",
  tplZenmux: "ZenMux",
  tplOpencodeGo: "OpenCode Go",
  tplVolcengine: "Volcengine Ark",
  fieldOrgId: "組織 ID",
  fieldProjectId: "專案 ID",
  fieldQuotaUrl: "用量端點 URL",
  fieldAccessKeyId: "AccessKey ID",
  fieldSecretAccessKey: "Secret AccessKey",
};
