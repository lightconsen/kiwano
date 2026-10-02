export const providers = {
  // ── Segment bar and list header ──
  all: "全部",
  counts: "{providers} 個 Provider · 已綁定 {agents} 個 Agent",
  addProvider: "新增 Provider",
  refreshTitle: "重新整理 Provider、方案配額與已安裝 Agent（略過 5 分鐘配額快取）",

  // ── Per-agent onboarding (empty tab) ──
  takenOverTitle: "{agent} 已接管",
  takenOverBody: "本機閘道正在轉發此 Agent 的請求，但尚未綁定任何 Provider —— 請新增一個，讓請求有地方可去。",
  startManaging: "開始用 Kiwano 管理 {agent}",
  startManagingBody:
    "Kiwano 會備份目前設定（之後可一鍵還原）、匯入 {agent} 已在使用的 Provider（與其他 Agent 共用），並透過本機閘道轉發 —— 上游不變，之後可即時切換。",
  enableKiwano: "啟用 Kiwano",
  enabling: "啟用中…",
  addProviderFirst: "先新增 Provider",
  oauthNote:
    "使用官方訂閱登入（Claude 登入、Codex ChatGPT）？官方 OAuth 目前無法代理 —— 請先手動新增一個 Provider。",

  // ── Provider rows ──
  unbound: "未綁定",
  inUse: "使用中",
  quotaFallback: "候補",
  quotaFallbackTitle:
    "主要 Provider 超出配額後第一個接手 —— 實際由哪個候補服務取決於其餘 Provider 是否仍可達",
  editProvider: "編輯 Provider",
  deleteProvider: "刪除 Provider",
  clickAgain: "再次點擊以確認",
  confirm: "確認",
  blocked: "已阻斷",
  notRoutingTitle: "未在此處路由：{reason}",

  // ── The Status column's measurements ──
  latencyTraffic: "該 Provider 最近 24 小時自身請求的平均延遲",
  latencyProbe:
    "可達性：閘道向該端點發了一次不帶憑證的請求 —— 它只說明那裡有東西回應，不代表你的 Key 可用 · 檢查於 {time}",
  latencyTest: "你測試過 —— 用該 Provider 的 Key 真的送出了一次請求 · 測量於 {time}",
  unreachable: "無回應",
  unreachableTitle: "上次詢問時該端點沒有回應 · 檢查於 {time}",
  refused: "金鑰被拒",
  refusedTitle: "端點有回應但拒絕了這個 Key：{error} · 檢查於 {time}",

  // ── Usage / quota cell ──
  usagePlanLimits: "方案 · 限額 {windows} · 以該 Provider 的方案配額使用率為準{tokens}",
  usageWindowFive: "5 小時時間窗 ≤ {percent}%",
  usageWindowWeekly: "每週時間窗 ≤ {percent}%",
  usageInOut: " · 輸入 {input} · 輸出 {output}",
  usagePlanQuota: "方案 · 本期 {used}/{limit} {unit}（{percent}%）· 重設於 {resets} · 輸入 {input} · 輸出 {output}",
  usageLocalInference: "本機推論 · 不計費 · 可離線使用",
  usagePaygQuota:
    "按用量計費 · 本期 {used} / {limit}（{percent}%）· 輸入 {input}（快取 {cache}，按 1/10 計費）· 輸出 {output} · 延遲 {latency}",
  usagePaygTrend: "按用量計費 · 未設定限額 · 近 7 天用量趨勢",
  planTemplateCached: "{template} · 已快取",
  planLimitFive: "5h ≤ {percent}%",
  planLimitWeekly: "週 ≤ {percent}%",
  unitReq: "次",
  unitTenKTok: "萬 tok",
  reqTokSuffix: "次 · {tokens} tok",
  inOutTokens: "輸入 {input} · 輸出 {output}",
  limitSuffix: "/ 限額 {limit}",
  tokensLatency: "{tokens} tokens · 延遲 {latency}",
  reqSuffix: "· {requests} 次",

  // ── Column headers ──
  colProvider: "Provider",
  colRole: "策略中的角色",
  colBoundAgents: "綁定的 Agent",
  colUsage: "用量 / 配額",
  colCache: "快取命中",
  cacheColTitle:
    "近 7 天提示快取命中率：快取讀取 ÷ 輸入側 tokens（新輸入 + 快取讀取 + 快取寫入）",
  cacheHitTitle: "近 7 天 {pct}% 的輸入側 tokens 來自快取（{read} / {denom}）",
  cacheNoData: "近 7 天沒有輸入側 tokens，無從計算命中率",
  colStatus: "狀態",
  colPriority: "優先順序",
  colActions: "操作",

  // ── Agent-tab role cell ──
  weight: "權重",
  weightTitle: "工作階段會依權重比例在各候選之間輪替",
  windowStart: "時間窗開始",
  windowEnd: "時間窗結束",
  windowTitle:
    "此候選生效的本機時間窗；直接輸入數字，如 0930 —— 結束早於開始即為跨午夜的時間窗。兩者都清空即可移除時間窗",
  fallback: "備援",
  fallbackTitle: "當所有候選的時間窗都不符合時使用",
  noWindow: "未設時間窗",
  noWindowTitle: "未設定時間窗 —— 此候選永遠不會被選中",
  primary: "主要",
  primaryTitle: "該 Agent 目前的路由",
  standby: "備用 #{index}",

  // ── Candidate row actions ──
  moveUp: "上移",
  moveDown: "下移",
  makePrimary: "設為主要",
  makePrimaryTitle: "設為主要 —— 將此候選移到佇列最前面",
  // ── Deleting a provider, and parking one instead ──
  delRemovedFrom: "將從 {agents} 中移除",
  delPromotes: "{provider} 將成為 {agent} 的主要",
  delEmpties: "{agent} 將沒有任何 Provider",
  disable: "停用",
  enable: "啟用",
  disableTitle: "停用：退出所有路由，但保留這一列與它的 Key",
  enableTitle: "讓它重新回到所綁定的路由裡",

  // ── Latency test (a prompt round trip, one per provider row) ──
  testLatency: "測試延遲",
  testLatencyTitle: "送出一則 prompt，測一次往返耗時",
  testLatencyFailed: "失敗",

  // ── User-defined agents (migration v16) ──
  // The + in the strip opens a menu rather than creating outright, so it needs its own
  // wording: the menu is "add an agent", and creating one is the first thing it offers.
  addAgentMenu: "新增 Agent",
  addAgentMenuTitle: "建立自訂 Agent，或手動指定 Kiwano 找不到的內建 Agent",
  newAgent: "建立自訂 Agent",
  // ── Pointing Kiwano at a built-in it could not find ──
  notDetected: "本機未偵測到",
  addAgentTitle: "新增 {agent}",
  declaredAgentTitle: "{agent} 的安裝目錄",
  installDir: "安裝目錄",
  installDirPlaceholder: "/opt/custom/bin",
  browse: "瀏覽…",
  searchedDirs: "已搜尋 {count} 個目錄，均未找到",
  checkingDir: "正在檢查該目錄…",
  dirOk: "找到可執行的檔案 — {version}",
  checkAgain: "重新偵測",
  recheckMissing: "剛剛又問了一次 —— 仍然沒有找到。",
  recheckNoAnswer: "剛剛又問了一次，但這次探測沒有回應。",
  checkAgainTitle: "現在再在這台機器上找一次",
  foundTitle: "找到了",
  foundBody:
    "它已經安裝在這台機器上，不需要再指定目錄 —— 工具列裡已經有它的分頁（就在這個對話框後面的 Apps 頁）。",
  addAgentNote:
    "指向存放它指令的目錄即可 —— Kiwano 只能確認那個指令可以執行，不能確認它就是該 Agent（各家的版本輸出格式不同，沒有可依賴的形狀）。",
  declaredAgentNote:
    "Kiwano 是依你指定的目錄找到它的。儲存時只會確認那個指令仍可以執行 —— 不能確認它就是該 Agent（各家的版本輸出格式不同，沒有可依賴的形狀）。",
  addAgent: "新增",
  alreadyDeclared: "已新增",
  editAgentName: "編輯 Agent 名稱",
  editAgentNote: "編輯備註",
  agentName: "名稱",
  agentNamePlaceholder: "長任務",
  agentNote: "備註",
  agentNotePlaceholder: "選填 —— 這條路由的用途",
  agentProtocol: "協定",
  agentProtocolUnset: "未指定",
  agentProtocolNote: "指向這條路由的用戶端所說的協定。僅作標註：路由、驗證與拒絕都不看它。",
  agentIdNote:
    "id 由名稱衍生且固定不變。磁碟上什麼都不會改：它是一條路由，所以你把用戶端指向閘道並帶上它的 Key，而不是去接管某個設定檔。",
  createAgent: "建立",
  routeEmptyTitle: "{agent} 還沒有候選 Provider",
  routeEmptyBody:
    "在下面綁定一個現有 Provider；帶著這個 Agent 的 Key 進來的請求，會依候選順序路由。",
  accessTab: "存取",
  accessEndpoint: "端點",
  accessKey: "API Key",
  accessNote:
    "任何用戶端指向這個端點、帶上這個 Key，就會依這個 Agent 的策略路由（/v1/messages 或 /v1/chat/completions）。",
  deleteAgent: "刪除這個 Agent",
  deleteAgentConfirm: "再點一次即刪除",
  deleteAgentNote: "它的用量歷史會保留。",


  // ── A built-in agent's own settings (the row above its table, and its dialog) ──
  agentSettingsFor: "{agent} 設定",
  agentGeneralTab: "一般",
  agentConfigFilesNote: "接管前備份，關閉接管時還原。",
  agentPlaceholderKey: "API Key",
  agentPlaceholderKeyTitle: "由閘道指派的預留金鑰，用於請求歸屬識別",
  agentAdditive: "共存 · 多 Provider",
  agentDisable: "關閉接管",
  agentDisableConfirm: "再次點擊以關閉",
  agentDisableNote: "還原該 Agent 自身的設定。路由與金鑰都會保留。",

  removeFromRouteAria: "從路由中移除",
  removeFromRoute: "從這條路由中移除（其他 Agent 的綁定不受影響）",

  // ── Bind-another-provider row ──
  bindAria: "將 Provider 綁定到路由",
  allBound: "所有 Provider 均已綁定",
  bindExisting: "綁定現有 Provider…",
  allInRoute: "所有 Provider 都已在目前的路由中 —— 請從頂端新增",
  bindAnother: "再綁定一個 Provider 到此路由 —— 它會作為備用加入佇列尾端",

  // ── Empty list ──
  none: "還沒有 Provider ——",

  // ── Plan quota tiers ──
  tierFiveHour: "5 小時時間窗",
  tierWeekly: "每週",
  tierMonthly: "每月",
};
