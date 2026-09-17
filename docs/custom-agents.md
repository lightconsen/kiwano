# 自定义 Agent（网关策略应用）设计（docs/custom-agents.md）

> 目标：让用户在 **Apps** 里自己增加「Agent」——它不必是某个真实的 CLI，而是
> **一条命名网关路由**：有自己的 key、自己的策略（strategy + 候选 provider 队列），
> 任何客户端把 base_url 指到网关并带上这个 key，就按这条策略走并归它记账。
> 现状基线：v0.1.9，锚点提交 `7b9db29`（2026-09-14）。文中 `file:line` 都按该提交核过。

## 0. 现状与约束

### 0.1 已经支持的部分（数据面不用改）

| 位置 | 事实 |
|---|---|
| `crates/gateway/src/router/mod.rs:145` | `RouteTable::load` 从 `placeholder_keys ∪ bound_agents` 建路由 —— **完全不查 agent 注册表**，任意 id 都能有路由 |
| `crates/gateway/src/router/mod.rs:272` | `route_agent` 只认 key：缺失/未知一律 401（**没有**按路径或协议回退） |
| `crates/gateway/src/store/mod.rs` | `agent_strategies` / `agent_bindings` / `placeholder_keys` / `usage` / `request_logs` 的 agent 列都是 TEXT，无 CHECK、无外键到注册表 |
| `crates/core/src/vm.rs`（`build_agent_routes`） | 遍历 `bound_agents()`，任意 id 都能组成标签页的路由 |

结论：**网关与存储层已经能跑自定义 agent**，缺的是「agent 列表」这一层把它当成一等公民。

### 0.2 挡住它的地方

| 位置 | 事实 |
|---|---|
| `crates/core/src/vm.rs:20` | `AGENTS: [(&str,&str); 8]` 是「有哪些 agent」的唯一来源 |
| `crates/core/src/vm.rs:2880` / `:2941` | Dashboard 的 `by_agent` 与 `filter_agents` **遍历 `AGENTS`** → 自定义 agent 的流量会整个漏掉 |
| `crates/core/src/vm.rs`（`live_bound_agents`） | 以「活配置文件里的占位 key」判定 agent 是否启用；自定义 agent 没有配置文件，会被判成休眠 |
| `crates/core/src/detect.rs:22` | `CLI_AGENTS` 是安装探测的固定表；前端按探测结果隐藏分段 |
| `app/src/api/types.ts:11` | `AgentId` 是闭集联合类型 |
| `app/src/screens/Providers.tsx:34` | 分段条（`SEGMENTS`）是写死的 8 项 + 图标 |
| `Providers.tsx:83` / `:561`、`components/StrategyPanel.tsx:64/181/183`、`api/dev.ts:482` | `AGENTS.find(...)!` 非空断言 → 未知 id 直接抛 |
| `app/src/App.tsx:49` | 深链 `#providers/<agent>` 用 `AGENTS.some(...)` 校验 → 自定义 agent 的深链会被丢弃 |
| `app/src/screens/AddProviderModal.tsx:1081/1102` | 「绑定 Agent」多选只列内置 agent |

复核方式（三条命令，不需要读全文）：

```sh
sed -n '145,175p' crates/gateway/src/router/mod.rs     # 路由表不查注册表
sed -n '272,293p' crates/gateway/src/router/mod.rs     # 只认 key，未知即 401
grep -rn "AGENTS" crates app/src --include=*.rs --include=*.ts --include=*.tsx
```

## 1. 概念与不变量

**内置 Agent = 路由 + 配置适配器；自定义 Agent = 只有路由。**

一个自定义 agent 拥有：id、显示名、占位 key（`kw-ag-<id>-<rand>`）、一条策略
（strategy + 候选 provider 队列）。它**不拥有**：配置文件、安装探测、导入来源、可还原备份。

写成不变量（实现时落进代码注释）：

1. **不进接管体系**：不参与 `detect` / `creds` / `import` / `takeover_paths` /
   `REBUILDABLE_AGENTS`。那些是「内置 agent 的配置改写」专用。
2. **不改写任何文件**：创建与删除自定义 agent 只动数据库；磁盘上什么都没发生。
3. **必须带自己的 key**：网关对未知 key 直接 401（`router/mod.rs:272`）。这是特性而非限制——
   key 就是身份，也正是它让「多个策略共用一个端口」成立。
4. **展示用途用并集，行为用途只用内置**：凡是「列出/展示 agent」的地方用
   `内置 ∪ 自定义`；凡是「按 agent 改写配置/探测」的地方只认内置。这条边界必须写进代码，
   否则下一次就会有人把自定义 id 喂给 `takeover_paths`。

## 2. 数据模型（迁移 v16）

```sql
-- crates/gateway/src/store/mod.rs：SCHEMA_VERSION 15 → 16
CREATE TABLE IF NOT EXISTS custom_agents (
    id         TEXT PRIMARY KEY,
    label      TEXT NOT NULL,
    note       TEXT,
    created_at TEXT NOT NULL
);
```

- **id**：`slug(label)` + `-` + 4 位 hex（与 provider 的 `<slug>-<hex>` 同规则，复用
  `vm::slug`）。规则与边界：
  - 与内置 8 个 id 冲突 → **拒绝**（保留字，不允许遮蔽真实 agent）；
  - 全中文/全符号名 slug 为空 → 退化为 `custom-<hex>`；
  - **id 生成后不随改名变**（bindings / strategy / keys / usage 都引用它）。
- **key**：创建时 `upsert_placeholder_key("kw-ag-<id>-<4hex>", id)`（API 已有，
  与接管铸 key 同一张表、同一个格式）。
- **删除**（语义对齐 provider 删除）：删 `custom_agents` 行 + 它的 key + `agent_bindings` +
  `agent_strategies`；**保留 `usage` / `request_logs`**（历史不改写；Dashboard 仍能查到，
  查不到标签时显示 id）。
- **不存图标/颜色**：从 label 派生（`logo_char` + `palette_color` 已存在），
  保持表最小。
- **P1 没有「停用」态**：删除即下线（`enabled` 列留到 P2），所以建表时也不要预留。
- 创建时同时写入一条默认策略 `Single`（与首次接管建路由的做法一致），
  使 `RouteTable::load` 立刻有它的路由。

## 3. 端到端行为

### 3.1 客户端怎么接

```
Endpoint  http://127.0.0.1:8317
API Key   kw-ag-my-route-3f9a
```

任何客户端（Claude Code、某 IDE 插件、一段脚本）把这两个值填进去即可：

- 路径决定**入站协议**：`/v1/messages` → Anthropic 形状，`/v1/chat/completions`（或 `/v1/responses`）→ OpenAI 形状；
  **两条都接受，P1 不做路径限制**（要收紧是后续的过滤器特性）；
- 候选 provider 的协议决定**是否转换**（Anthropic↔OpenAI 转换已有）；
- 归属由 key 决定，因此同一条策略可以被任意多个客户端共用。

### 3.2 网关侧为什么不用改

`RouteTable::load`（`router/mod.rs:145`）已经把 `placeholder_keys` 里的每个 agent 都建了路由，
策略与候选来自 `agent_strategies` / `agent_bindings`——三张表都不认识「注册表」。
要做的只是让应用层**创建这三行**（表行 + key + 默认策略）。

## 4. 改动清单

| 层 | 改动 | 位置 |
|---|---|---|
| 迁移 | `MIGRATION_V16`、`SCHEMA_VERSION = 16` | `crates/gateway/src/store/mod.rs` |
| store | `list_custom_agents` / `insert_custom_agent` / `update_custom_agent` / `delete_custom_agent` | 同上 |
| core vm | `add_custom_agent(store, aux, label, note)`（建行 + 铸 key + 默认 Single 策略）、`remove_custom_agent(store, id)`（级联同上）、`list_agents(store)`（内置 ∪ 自定义） | `crates/core/src/vm.rs` |
| core vm | `live_bound_agents`：**自定义 agent 恒为 live**（它没有配置文件可读，否则会重现「All 页显示未绑定」的假象） | `crates/core/src/vm.rs` |
| core vm | Dashboard：`by_agent` 与 `filter_agents` 的行集合改为取自 usage 的 distinct agent，标签查「内置 ∪ 自定义」 | `vm.rs:2880` / `:2941` |
| core vm | `SettingsVm.custom_agents: Vec<CustomAgentVm { id, label, note, placeholder_key }>` —— **不塞进 `takeovers`**：那里是「我们改写了它的配置」的诚实语义，自定义 agent 没有配置 | `crates/core/src/vm.rs` |
| CLI | `kiwano agents add --name <名字> [--note …]`、`agents remove <id>`、`agents list`（标注 custom）；绑定走**已有的** `routes binding add <agent> <provider>` —— `routes` / `strategy` / `binding` 本来就吃任意 agent 字符串，一行都不用改 | `crates/cli/src/cmds.rs`、`cli.rs` |
| 前端 types | `AgentId` 拆成 `BuiltinAgentId`（闭集：AGENTS、图标、接管用）+ `AgentRef = string`（路由/日志/用量等边界）；新增 `agentMeta(id)` 解析器（内置→registry；否则→派生：名称首字母 + 调色板色） | `app/src/api/types.ts` |
| 前端 | 分段条尾部 `+`；自定义 tab（无接管态）；接入信息卡；删除；用 `agentMeta` 替换所有 `AGENTS.find(...)!`；深链校验接受自定义 id；AddProviderModal 多选列出并集 | `Providers.tsx`、`StrategyPanel.tsx`、`App.tsx`、`AddProviderModal.tsx` |

**不需要改**：`crates/gateway/src/router/**`、`forward/**`、`meter/**`、`limits.rs`、
`crates/core/src/detect.rs`、`takeover.rs`、`creds.rs`、`import.rs`。

## 5. UI 设计

### 5.1 入口

分段条（agent 切换器）最右端一个 `+` → 弹窗「自定义 Agent」：

```
┌─ 自定义 Agent ─────────────────────────┐
│ 名称      [长任务默认路由          ]    │
│ 备注      [比如：便宜优先 / 夜间跑批 ]  │  ← 可选
│                         [取消] [创建]   │
└────────────────────────────────────────┘
```

用户**只填名字**（id 自动派生，不在表单里出现）。创建后直接切到它的 tab，界面把派生的 id 与 key 一并给出来；
候选 provider 在那个 tab 里绑（空态就指向这件事）。

### 5.2 它的 tab

与内置 agent tab 同构（策略面板 + 候选队列 + 上下移/固定/解绑），差别只有两处：

- **没有接管态**：不出现 Enable / 还原 / 重写配置的任何入口；
- 顶部多一张**接入信息卡**（整个特性的价值所在）：

```
┌─ 接入 ────────────────────────────────────────────────┐
│ Endpoint   http://127.0.0.1:8317              [复制]  │
│ API Key    kw-ag-my-route-3f9a                [复制]  │
│ 任何客户端把 base_url 指到这里、带上这个 key，就按这条  │
│ 策略走（/v1/messages 或 /v1/chat/completions）。       │
└───────────────────────────────────────────────────────┘
```

- **空态**：`还没有候选 Provider → 绑定一个`（复用现有 onboarding 的 bind slot，
  去掉 Enable 那半边，内部标记 `kind: "route-only"`）。
- **删除**：tab 内两步确认（与 provider 行一致）；删除后段条上消失，
  Dashboard 的用量历史保留（标签缺失时显示 id）。P1 没有停用态——下线就是删除（§7 #2）。

### 5.3 连带屏幕

- **Dashboard / Logs**：agent 筛选与 `by_agent` 自然出现它（依赖 §4 的集合修复）。
- **Settings**：接管列表**不出现**它（那页是配置改写）；自定义 agent 的 key 也不在这里显示，
  只在其 tab 与 CLI 里给。
- **Apps 的 All 页**：provider 的 Bound-agents 列会出现它的字母头像（内置走品牌图标，自定义走字母头像）。

## 6. 风险与边界（实现时逐条验证）

1. **live 判定**：`live_bound_agents` 若不给自定义 agent 开例外，All 页会把它显示成
   「未绑定」——这与之前修过的「休眠路由冒充在用」是同一类假象，最容易漏。
2. **未知 id 的渲染兜底**：自定义 agent 的存在让「未知 id」从异常变成常态；
   所有 `AGENTS.find(...)!` 必须换成 `agentMeta`。
3. **深链**：`#providers/<custom-id>` 必须能进（`App.tsx:49` 的校验要放宽）。
4. **id 规则**：保留字、空 slug、重复 id 三条要明确且可测。
5. **概念边界**：注释里写清「展示用途用并集、行为用途只用内置」，
   并在 `takeover_paths`/`detect` 的入参类型上体现（内置 id 类型 vs 任意 agent 引用）。
6. **Dashboard 的口径**：自定义 agent 的流量必须出现在 `by_agent`（否则用户在 Dashboard 上看不到
   自己的策略花了多少），但**不**新增任何限额语义（provider 级限额已经生效）。

## 7. 已决策（2026-09-14）

| # | 决策 | 结论 |
|---|---|---|
| 1 | id 生成 | **用户只填显示名，id 自动派生 `slug-<4hex>`**（与 provider 同规则；id 生成后不随改名变） |
| 2 | 停用 vs 删除 | **只做删除**（无 `enabled` 列）；`enabled` 开关留到 P2 再说 |
| 3 | 入站路径限制 | **不限制**：客户端走 `/v1/messages` 或 `/v1/chat/completions` 都可以，路径决定协议 |
| 4 | 创建即绑定（CLI `--bind` / 弹窗「立即绑定」多选） | **不做**（P1 分两步：先建路由，再在 tab 里绑；一步到位是 P2）。理由：`providers add --bind` 只是便利，不绑也能用；而 `routes binding add` 已经存在，命令行不必为它加新 flag |
| 5 | agent 级限额 | **不做**（provider 级限额已生效）；P3 另开特性 |

## 8. 分期与验收

### P1（可用）

迁移 v16 + store CRUD → core vm（创建/删除/`list_agents`/live 口径/Dashboard 集合/Settings 交付）
→ CLI 三条命令 → 前端（`+`、tab、接入卡、删除、`agentMeta` 兜底、段条恒显、Modal 多选、深链）。

**Rust**
- 创建：id 派生、保留字拒绝、空 slug 退化；重复名得到不同 id。
- 删除：key / bindings / strategy 清空；usage / request_logs 保留。
- `live_bound_agents`：自定义 agent 的 provider **不会**在 All 页显示为未绑定。
- `build_dashboard`：它的流量出现在 `by_agent` 与 `filter_agents`。
- `RouteTable::load`：含它的路由；`build_settings` 交付形状（含 key）。

**CLI e2e**
- `agents add` → `routes binding add`（现有命令）→ **伪造一条带该 key 的请求走通并记为它的用量** → `agents remove` 后同 key 401。

**前端**
- 创建后段条出现该 tab、tab 内**无** Enable；
- 接入信息卡可复制；
- 删除后段条消失；
- 未知 id 渲染不崩（`agentMeta` 兜底）；
- `#providers/<custom-id>` 深链能进。

### P2（打磨）

客户端片段一键复制（curl / Claude Code / 通用 base_url 填法）、自定义 agent 的导出与导入
（把一条策略分享给别人）、`enabled` 开关、**创建即绑定**（CLI `agents add --bind …` 与弹窗多选，二者或其一）。

### P3（能力）

agent 级限额、策略模板（预置「coding plan 优先」「夜间便宜」等常见组合）。

## 9. P1 实现状态（2026-09-14，已完成）

按 §8 落地，与设计一致的部分不再重复；**与设计有出入的三处**记在这里：

1. **Dashboard 的 agent 行集合比设计稍宽**：除了「内置 ∪ 自定义」，还并入了**只在 usage 里有流量**
   （`store::usage_agents`）的 agent。原因：删除一个自定义 agent 后它的用量记录仍在，而那些行
   没有名字可查 —— 不并入就等于把历史从页面上抹掉。标签缺失时显示 id。
2. **`build_settings_with_home` 又收回了 `store` 参数**（本会话早前刚把它去掉）。当时它确实不需要
   存储层；现在要读 `custom_agents` 表，就又需要了 —— 这是数据在存储层、而不是在 `aux` 设置
   blob 里的一处必然结果。
3. **CLI 的 `agents list` 是新命令**（此前只有 detect/versions/takeover/restore）。它合并
   `takeovers`（内置：是否已接管）与 `custom_agents`（自定义：永远 routed）成一张表，并打印 key ——
   对服务器用户来说，那个 key 就是全部接入信息。

实现要点（供后续改动参照）：
- 迁移 `MIGRATION_V16`，`SCHEMA_VERSION = 16`；表 `custom_agents(id, label, note, created_at)`。
- `vm::add_custom_agent` 写三行：表行 + `kw-ag-<id>-<rand>` 占位 key + 默认 `single` 策略；
  `vm::remove_custom_agent` 反向清三行（usage/request_logs 保留）。
- `vm::live_bound_agents` 对自定义 agent **恒判 live**（否则 All 页会把它显示成未绑定）。
- 前端 `lib/agents.ts`：`agentMeta(id)` 解析器（内置 → registry，自定义 → 用户标签，未知 → 派生
  字母头像），`isBuiltinAgent(id)` 是**类型守卫** —— 接管方向因此拿不到自定义 id。
- 所有 `AGENTS.find(...)!` 已替换（它们是「未知 id 就崩」的隐患，而自定义 agent 让未知 id 成为常态）。
- 测试：Rust 3 条（vm）+ 1 条（gateway 集成：带 key 的请求走通并记账）+ CLI 2 条（add/list/bind/remove
  与空名拒绝）+ 前端 7 条（段条、接入卡、空态、创建、删除、未知 id 兜底、mock 语义）。

## 10. 协议标识（迁移 v24，2026-09-17）

### 10.1 记录的是什么

在这之前，「一个 agent 说什么协议」没有被记在任何地方，只被两处间接表达：

- **内置 agent**：`takeover::gateway_target` 按 agent 决定交给网关的 URL 长什么样（`/v1`、
  `/v1/chat/completions` 或空），各家 rewriter 各自知道该往配置里写 `wire_api = "responses"`、
  `api: "openai-completions"`、`type = "openai"`。
- **自定义 agent**：一条命名路由加一个 key，什么协议都没说。

这一轮把它显式化：**内置 agent 带一份协议列表，自定义 agent 创建时选一个。**
取值表（依据是各家 rewriter 写进配置的线格式 —— 那是工具真正读的东西）：

| agent | 协议 | 依据 |
|---|---|---|
| claude | anthropic | `settings.json` 的 `ANTHROPIC_BASE_URL` |
| codex | openai | `config.toml`：`wire_api = "responses"` + bearer token |
| gemini | gemini | `~/.gemini/.env` 的 `GEMINI_API_KEY`；`/v1beta/models/{model}:{action}` |
| grokbuild | openai | `api_backend = "responses"` |
| claude-desktop | anthropic | 3p profile：`inferenceGatewayBaseUrl` + `anthropic/claude-*` 路由 |
| opencode | openai | 条目 `npm: "@ai-sdk/openai-compatible"` |
| openclaw | openai | 条目 `api: "openai-completions"` |
| hermes | openai | `custom_providers` 的 base_url/api_key |
| pi | openai | 条目 `api: "openai-completions"` |
| workbuddy | openai | 条目 `api: "openai-completions"` |
| codebuddy | openai | 同上 |
| kimi | openai | `providers[].type = "openai"` |
| qwen | openai | `modelProviders.openai` + `security.auth.selectedType = "openai"` |
| cline | openai | `~/.cline/data/settings/providers.json`：`settings.provider = "openai-compatible"` + 它自己的 `baseUrl` |

**Cline 指的是 Cline CLI，不是 VS Code 扩展。** 扩展的配置在 VS Code 的 globalStorage
（`saoudrizwan.claude-dev/state.vscdb`）与操作系统 keychain 里，那份状态还在迁移中 —— 不是能安全
备份、改写、还原的东西，所以扩展只能按 §1 的自定义 agent 手接（Endpoint + key 自己填）。CLI 有
`~/.cline/` 下的磁盘配置，才落进接管体系。

`pi` 与 `qwen` 两行**按 `openai` 定**（2026-09-17）。它们的依据只有「Kiwano 接管时写什么」这一条，
没有独立核实过它们除此之外还接受什么 —— 尤其是 `qwen` 是 gemini-cli 的 fork，原生协议很可能
也吃 `gemini`。记在这里是为了让后来的人知道这两行的证据强度不如其余十一行：将来真要做校验时，
它们是先要复核的两行，而不是可以直接当地基的两行。

### 10.2 边界：只是标识

**不参与网关的协议判定。** 入站协议仍由路径决定（`gateway::protocol::classify_path`），
归属仍由 key 决定（`router::route_agent`）。这一条与 §1 的第 4 条同源：它是「关于 agent 的事实」，
不是「关于路由的配置」。

因此这一轮**明确不做**：

- 不改 `gateway/src/protocol.rs` 的路径判定、`resolve_inbound`、路由表、provider 的 `protocol` 列；
- **不做校验**：不会因为「agent 声明 anthropic、绑的 provider 是 openai」就警告或拒绝
  （跨协议转换本来就存在，声明与转换不矛盾）；
- 不改探测：`CLI_AGENTS`（二进制名）与协议无关。

### 10.3 机制

| 层 | 改动 |
|---|---|
| 迁移 | `MIGRATION_V24`：`ALTER TABLE custom_agents ADD COLUMN protocol TEXT`（**不加 CHECK** —— 按 v19 的教训，一个要扩展就得先丢掉的约束只换来一次迁移）；`SCHEMA_VERSION 23 → 24` |
| store | `CustomAgent.protocol: Option<String>`，两条 SELECT / INSERT / UPDATE 显式带列；`None` = 未指定（迁移前建的行） |
| core vm | `AGENT_PROTOCOLS: [(&str, &[&str]); 13]` —— 按 id 索引的平行表（`AGENTS` 被 `(agent, label)` 解构十几处，不动它的元数），配 `agent_protocols(id)` 查询与「每个 AGENTS id 都在这张表里且非空」的不变量测试 |
| core vm | `TakeoverVm.protocols: Vec<String>`（与 `additive` 一样：关于 agent 的事实，借那一行交付）、`CustomAgentVm.protocol: Option<String>` |
| Tauri / CLI | `add_custom_agent` / `update_custom_agent` 多一个参数；`agents add --protocol <P>`，`agents list` 文本表多一列 PROTOCOL、`--json` 带 `protocols` |
| 前端 | 创建对话框选协议（默认 openai）；接入页一行可改；内置 agent 的齿轮对话框**陈述**它（只读，那里本来就在陈述它的配置文件路径）；未指定显示「未指定」 |

### 10.4 一次更新的口径

`update_custom_agent` 替换该 agent 自己的全部三个字段（名称、备注、协议），不是打补丁：
`null` 是协议可以持有的值（「没说」），所以调用方要把**自己不改的字段原样传回**。
前端接入页的两个控件（改名、选协议）都照此办理 —— 同名同姓的两处调用各传另外两个字段。

### 10.5 验收

- Rust：v24 迁移测试（手工重放 V7..V23、stamp 23、开库，断言旧行还在且 `protocol IS NULL`，再写一行带协议）、
  `AGENTS ↔ AGENT_PROTOCOLS` 不变量、四个自定义 agent 测试与 gateway 集成的结构体字面量。
- 前端：创建对话框（选协议 → `addCustomAgent` 收到第三个参数）、接入页显示/改协议、
  「未指定」的读法、i18n 键集平价。
- 端到端：`kiwano agents add --name muse --protocol gemini` 后 `agents list --json` 里是 `gemini`；
  旧的 custom agent 仍在、协议为「未指定」。
