// providers — Simplified Chinese. The key set is not decided here: `en/` is
// the source of truth and this file must match its shape (see ../types.ts).
export const providers = {
  // ── Segment bar and list header ──
  all: "全部",
  counts: "{providers} 个 Provider · 已绑定 {agents} 个 Agent",
  addProvider: "添加 Provider",
  refreshTitle: "刷新 Provider、套餐配额与已安装 Agent（绕过 5 分钟配额缓存）",

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
    "使用官方订阅登录（Claude 登录、Codex ChatGPT）？官方 OAuth 目前无法代理 —— 请先手动添加一个 Provider。",

  // ── Provider rows ──
  unbound: "未绑定",
  inUse: "使用中",
  quotaFallback: "候补",
  quotaFallbackTitle:
    "主 Provider 超出配额后第一位接手 —— 实际由哪个候补服务取决于其余 Provider 是否可达",
  editProvider: "编辑 Provider",
  deleteProvider: "删除 Provider",
  clickAgain: "再次点击以确认",
  confirm: "确认",
  blocked: "已阻断",
  notRoutingTitle: "未在此处路由：{reason}",

  // ── 状态列的两个测量来源 ──
  latencyTraffic: "该 Provider 最近 24 小时自己请求的平均延迟",
  latencyProbe:
    "可达性：网关向该端点发起了一次不带凭据的请求 —— 它只说明那里有东西应答，不校验你的 Key · 检查于 {time}",
  latencyTest: "你测试过 —— 用该 Provider 的 Key 真发了一次请求 · 测量于 {time}",
  unreachable: "无响应",
  unreachableTitle: "上次询问时该端点没有应答 · 检查于 {time}",
  refused: "密钥被拒",
  refusedTitle: "端点有应答但拒绝了这个 Key：{error} · 检查于 {time}",

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
  // ── 删除 provider，以及用停用代替删除 ──
  delRemovedFrom: "将从 {agents} 中移除",
  delPromotes: "{provider} 将成为 {agent} 的主用",
  delEmpties: "{agent} 将没有任何 Provider",
  disable: "停用",
  enable: "启用",
  disableTitle: "停用：退出所有路由，但保留这一行与它的 key",
  enableTitle: "让它重新回到所绑定的路由里",

  // ── 延迟测试（每个 provider 行一次 prompt 往返）──
  testLatency: "测试延迟",
  testLatencyTitle: "发一条 prompt，测一次往返耗时",
  testLatencyFailed: "失败",

  // ── 自定义 Agent（迁移 v16）──
  // 工具条上的 + 现在打开菜单而不是直接新建,所以它要有自己的措辞:
  // 菜单是「添加 Agent」,新建自定义 Agent 是它给出的第一项。
  addAgentMenu: "添加 Agent",
  addAgentMenuTitle: "新建自定义 Agent,或手动指定 Kiwano 找不到的内置 Agent",
  newAgent: "新建自定义 Agent",
  // ── 手动指定 Kiwano 找不到的内置 Agent ──
  notDetected: "本机未检测到",
  addAgentTitle: "添加 {agent}",
  declaredAgentTitle: "{agent} 的安装目录",
  installDir: "安装目录",
  installDirPlaceholder: "/opt/custom/bin",
  browse: "浏览…",
  searchedDirs: "已查找 {count} 个目录,均未找到",
  checkingDir: "正在检查该目录…",
  dirOk: "找到可运行的执行文件 — {version}",
  checkAgain: "重新探测",
  checkAgainTitle: "现在再在这台机器上找一次",
  foundTitle: "找到了",
  foundBody:
    "它已经装在这台机器上,不需要再指定目录 —— 工具条里已经有它的标签页(就在这个弹窗后面的 Apps 页)。",
  addAgentNote:
    "指向存放它命令的目录即可 —— Kiwano 只能确认那个命令可以运行,不能确认它就是该 Agent(各家的版本输出格式不同,没有可依赖的形状)。",
  declaredAgentNote:
    "Kiwano 是按你指定的目录找到它的。保存时只会确认那个命令仍然可以运行 —— 不能确认它就是该 Agent(各家的版本输出格式不同,没有可依赖的形状)。",
  addAgent: "添加",
  alreadyDeclared: "已添加",
  editAgentName: "编辑 Agent 名称",
  agentName: "名称",
  agentNamePlaceholder: "长任务",
  agentNote: "备注",
  agentNotePlaceholder: "可选 —— 这条路由是干什么的",
  agentIdNote:
    "id 由名称派生且固定不变。磁盘上什么都不会改：它是一个路由，所以你把客户端指到网关并带上它的 key，而不是去接管某个配置文件。",
  createAgent: "创建",
  routeEmptyTitle: "{agent} 还没有候选 Provider",
  routeEmptyBody:
    "在下面绑定一个已有 Provider；带着这个 Agent 的 key 进来的请求，会按候选顺序走。",
  accessTab: "接入",
  accessEndpoint: "端点",
  accessKey: "API Key",
  accessNote:
    "任何客户端指到这个端点、带上这个 key，就按这条 Agent 的策略走（/v1/messages 或 /v1/chat/completions）。",
  deleteAgent: "删除这个 Agent",
  deleteAgentConfirm: "再点一次即删除",
  deleteAgentNote: "它的用量历史会保留。",


  // ── 内置 agent 自己的设置（表格上方那一行，以及它的对话框）──
  agentSettingsFor: "{agent} 设置",
  agentGeneralTab: "通用",
  agentConfigFilesNote: "接管前备份，关闭接管时还原。",

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

  // ── Plan quota tiers ──
  tierFiveHour: "5 小时窗口",
  tierWeekly: "每周",
  tierMonthly: "每月",
};
