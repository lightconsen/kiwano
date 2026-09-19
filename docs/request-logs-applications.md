# request_logs 的应用方向（docs/request-logs-applications.md）

> 目标：网关手里的 `request_logs` + `request_bodies` 目前只被三个消费面读取
> （Logs 页看原文、Dashboard 算钱、`kiwano insights` 出报告），全是"事后给人看"。
> 这篇把可能的应用发散一遍并给出优先级判断——**是机会地图，不是排期承诺**。
> 现状基线：v0.1.15，锚点提交 `caa70d0`（2026-09-18）。文中引用的列与表都按该提交核过。

## 0. 手里到底有什么

这份数据的独特性在于：**网关是中间人，既有元数据又有完整 body，还有路由控制权**。

| 数据 | 位置 | 关键字段 |
|---|---|---|
| 请求元数据 | `request_logs`（30 列） | `ts / agent / attribution / provider_id / model / status_code / error_kind / session_id / is_streaming / input / output / cache_read / cache_creation / reasoning_tokens / usage_missing / latency_ms / first_token_ms / request_size / cost*` |
| 完整报文 | `request_bodies` | `request_body / response_body`（capture 开关下全量存，超限截断标 `truncated`） |
| 聚合 | `usage`（`usage_bucketed` 按小时/天现场分桶） | 按 provider×agent×时间的钱与 tokens，Dashboard 的现成数据源 |
| 控制权 | router / strategy / takeover | 路由选择、候选队列、配置文件改写通道——分析结论有能力变成动作 |

## 1. 被动分析（数据已在库里，纯读取）

### 1.1 会话回放 / 对话导出

`request_bodies` 可以把一个 session 还原成可读的 markdown/JSONL：
"这个 agent 那晚到底干了什么"。审计、复盘、排查"它为什么改了这个文件"都靠它。
纯本地读取，隐私承诺不破。已有 `logs export --include-bodies` 是它的原料形态，
缺的是**按会话组织、还原对话结构**的那一层。

### 1.2 Provider 速度榜

`latency_ms` + `first_token_ms` 按 provider×model 算 P50/P95，就是一张真实竞速表。
比任何评测站都准，因为是**用户自己的流量、自己的网络环境**。
Apps 页状态列已经用了"最近 24h 自己请求的平均延迟"，这是它的完全体
（分位数、按模型分组、跨 provider 对比）。

### 1.3 成本预测

> 2026-09-19 已落地（Features `feat_cost_forecast`）：货币型周期限额的
> 消费斜率外推，超限前通知；周期前 10% 的噪声段不报。

`usage_hourly` 有按小时聚合，外推"照这个速度月底花多少"，在超预算前告警。
alerts 基础设施已有（配额阈值 + 桌面通知 + dedup），只是判据从"配额已超"
变成"按当前斜率配额将超"。

### 1.4 异常检测

> 2026-09-19 已落地（Features `feat_anomaly_alerts`）：最近一小时 vs 过去
> 7 天基线（新增 store::traffic_stats），错误率/延迟/流量三条规则，
> 每条每天最多通知一次。

错误率突增、延迟离群、某 agent 半夜流量暴涨 → 桌面通知。
真实案例：库里 2026-09-09 那场 14 秒 19 连发的 `protocol_mismatch`
（`kiwano insights` 的第一条 finding），理想状态是**当时**就弹通知，
而不是事后跑报告才看到。

## 2. 反馈回路（把结论送回 agent）

insights 设计时按侵入性排过序（人读报告 → agent 自查询 → 配置注入 → 请求侧注入），
第 0 层（人读报告）已落地，剩下三层：

### 2.1 MCP tool / skill 自查询

> 2026-09-19 已落地（Features `feat_mcp_self_query`）：`kiwano mcp` stdio
> server（手写换行 JSON-RPC，无协议依赖），tools = get_usage_summary /
> get_insights / get_session_growth，全部只读聚合。

把**聚合数**（不是原文）包成 MCP tool：agent 问"我这周命中率多少、哪个会话在膨胀"。
Claude Code 这类 agent 看到"你的上下文已 38×"会自己 compact。
中等侵入，但它消费的是统计值，body 不出网关。

### 2.2 接管通道注入规则

> 2026-09-19 已落地（Features 面板 `feat_rule_injection`）：`kiwano rules
> apply <agent>` 把 insights finding 的 `rule` 文本写进 CLAUDE.md/AGENTS.md
> 的 marker 块，`remove` 剥离、`status` 查看。注意它不在 takeover 状态机里
> （takeover 只改 agent 配置文件，从不写 CLAUDE.md——本段此前的说法与代码
> 不符）；备份复用 takeover_backups 表（key 前缀 `rules:`），归因靠
> `rules_applied:<agent>` 时间戳，注入前后各跑一周 insights 即可对比。

### 2.3 自动调参

> 2026-09-19 落地的是**建议形态**（Features 面板 `feat_tuning_advice`）：
> insights 新增两条 `tuning` finding——路由健康（某 provider 错误率显著高于
> 同 agent 候选）与重试预算（重试几乎从不救回请求）。闭环改配置未做：
> 权重/重试预算是静态 DB 配置，运行时唯一的自适应是熔断器（二值），
> 连续调权需要先回答"改错了怎么归因"。

原设想：日志知道哪些 `error_kind` 重试能成、几次能成 → 每个 provider 的重试预算自动收敛；
哪条路由错误率高 → 策略权重自动降。把 strategy 的静态配置变成闭环控制。

### 2.4 请求侧注入：维持"不建议"

往 body 里塞 system reminder 会坐在前缀里，一改全仓缓存击穿，
且改变 agent 行为后无法归因。insights 设计时否掉，这里维持原判。

## 3. 主动介入（网关不只是记录，而是改变流量）

### 3.1 缓存整形代理 ⭐ 价值最大的一条

> 2026-09-19 落地了第一步（Features `feat_cache_experiment`）：
> `kiwano cache-experiment` 按 session 对比相邻 turn 的公共前缀重合度
> （原始 vs key 排序 vs 白名单剔除），纯只读。转发路径未动。
> 本机首跑结论：too little data——session 捕获自 v0.1.15 才有，攒几天再跑。

命中率低的原因是"前缀在抖"（tools 重排、易变字段混进 system）。
网关可以**在转发前把 body 规范化**——JSON key 排序、剔除白名单内的易变字段——
让本来失效的前缀缓存命中。agent 无感，token 直接省。

风险与前提：必须保证语义不变，只能白名单式保守规则。
**先做离线实验再决定**：拿库里已有的 body 对比"规范化后相邻请求的公共前缀重合度
能提升多少"，用数据说话。

### 3.2 切换前兼容性验证

用录制的真实请求重放到新 provider，切换前就知道会不会 `protocol_mismatch`。
09-09 那场风暴的本质就是"客户端对着不兼容的端口打"，这个功能把它变成
切换前的一次预检（provider 编辑对话框里加一个"用最近一条真实请求试打"）。

### 3.3 按 token 预算限额

> 2026-09-19 核对：**已实现**（agent_limits 表 + limits::evaluate 30s 求值 +
> 策略引擎强制拦截，支持 requests/wan_tokens/货币 × day/weekly/monthly/yearly）。
> 当时唯一的缺口——agent 被限额拦截时没有通知——已由 Features 面板的
> `feat_agent_limit_alerts` 补上。

每条自定义 agent 路由设"每天 1M tokens"，网关按日志/usage 累计强制执行。
核心卖点 limiting 的自然延伸：现在的限额是 provider 配额维度的，这是 agent 维度的。

## 4. 数据资产（更远）

### 4.1 个人评测集

失败/重试的请求天然是难例，攒起来就是针对自己工作流的 eval 集，换模型前先跑一遍。
依赖 §5 的结果标签补齐，否则"难例"没有对错可判。

### 4.2 团队分账

`attribution` 列已存在（`key` / `path_fallback`），按项目/人归集成本，导出账单。
单机产品先不急，团队版再说。

### 4.3 匿名遥测（谨慎）

opt-in 的匿名聚合（错误种类分布、命中率分布）能反哺 catalog 与路由默认值。
与"bodies and keys never leave this machine"的承诺冲突面大，
**默认不做**；若做必须只出不进原文、且逐项 opt-in。

## 5. 数据缺口（做之前要先补采集的）

| 缺什么 | 挡住了谁 | 备注 |
|---|---|---|
| 结果标签（任务成没成） | §4.1、所有"效果"归因 | 日志里只有请求没有结果；真要标签得从 agent 侧（hooks）拿，网关自己造不出来 |
| 流内 token 间隔 | 流式质量的细分析 | 现有 `first_token_ms` 只覆盖首包 |
| session_id 覆盖率 | §1.1、一切会话级指标 | 列自基础 schema（v5）就有，但网关的捕获（`session_hint`，server/data.rs）是 `528cb74`（2026-09-18）才接上的——之前的历史行（含本机那 24 行）全空。新行读取顺序：`x-kw-session` 头 → 各 agent 自己的 session 头 → Anthropic `metadata.user_id` → Codex `client_metadata.session_id` → `prompt_cache_key` → 顶层 `session_id` → 会话开头指纹（`derived:` 前缀，system + 首条 user 消息的 hash，给 Pi 这类线上不带 session 字段的客户端兜底）。Codex 头和 body 两样都发（本机实测同一个 UUID）；`derived:` 行是网关的推测而非客户端的自述，分析时可区分 |
| body 截断率 | §3.1 的离线实验样本量 | `truncated=1` 的行不能用于前缀分析 |

## 6. 优先级判断

按"数据已就绪 × 价值密度"排：

1. **先做**：§1.2 速度榜、§1.1 会话导出——纯读取、量级小、立刻有用。
2. **先实验后决定**：§3.1 缓存整形——价值最大，但先跑离线前缀重合度实验，
   数据支持才立项。
3. **等一等**：§2.1 MCP 自查询——让 insights 先跑一两周，确认这些指标
   真的指导了行为，再把它交给 agent。
4. **顺手**：§1.3 / §1.4 复用 alerts 基础设施，判据加两条而已。
5. **不动**：§2.4 请求侧注入（维持否决）、§4.3 遥测（默认不做）。

## 7. 已有消费面（避免重复建设的索引）

| 消费面 | 读什么 | 位置 |
|---|---|---|
| Logs 页 / `kiwano logs` | 元数据 + 单行 body | `store::list_request_logs` / `get_request_log` |
| CSV 导出 | 元数据 ± body | `core::csv` / `store::export_request_logs_with_bodies` |
| Dashboard 的钱与趋势 | `usage` / `usage_hourly`（不直接读 logs） | `vm::build_dashboard` |
| Apps 状态列延迟 | 24h 均值（logs 聚合） | `vm` 的 provider health |
| `kiwano insights` | 全窗口元数据 + 每 agent 一条 body 采样 | `core::insights`（caa70d0） |
