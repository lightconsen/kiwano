# Kiwano 桌面端 UI 设计规范 (Design Notes)

- 状态：v0.2 与 `design/index.html` 原型同步 | 更新日期：2026-09-07
- 原型：`design/index.html`（单文件，Tailwind CDN + lucide，hash 路由）

## 0. 设计原则（来自用户约束，必须遵守）

1. **桌面工具，不是网页** —— 无营销 hero、无大留白、无货架式首屏
2. **首屏 = 本机 Provider 列表**：每行展示 供应商 / 绑定的 Agent / 近 7 日用量 / 状态 / 操作
3. **窗口 1000×650**（对齐 cc-switch，min 900×600），macOS Overlay 标题栏（红绿灯叠在内容上）
4. **紧凑密度**：行高 ~56px、字号 12.5-13px、窄边距；顶部 tab 切换，不用侧边栏

## 1. 布局骨架

```text
┌────────────────────────────────────────────────────────────┐
│ ●●●  [🥝 Kiwano]  我的供应商 | 货架 | 仪表盘 | 设置   ●网关·运行 │ 46px 标题栏(Overlay)
├────────────────────────────────────────────────────────────┤
│ [全部|Claude Code|Codex|Gemini CLI]        [+ 添加供应商]   │ 44px 工具栏
│ 供应商      绑定的Agent      近7日用量     状态   操作       │ 28px 列头
│ ┃● DeepSeek  Claude·Codex   12,402次 $8.4  正常   编辑 启用 │ 56px 行(current左kiwi条)
│  Kimi·月之暗面 Claude        2,866次 $1.2   备用#1 编辑     │
│  GLM·智谱      —             —            未绑定 编辑     │
│  Ollama·本地   Codex         341次  $0     本地   编辑     │
├────────────────────────────────────────────────────────────┤
│ 今日请求 1,240 次 · Hub 同步 12:04 · v0.1.0                  │ 26px 状态栏
└────────────────────────────────────────────────────────────┘
```

## 2. 屏幕清单（hash 路由）

| # | 屏幕 | hash | 说明 |
| --- | --- | --- | --- |
| 1 | 我的供应商（首屏） | `#providers` | 列表模式；agent 过滤段控件（点击按 data-agents 过滤行，空结果显示引导态）；DeepSeek 为当前项（kiwi 左条+高亮） |
| 2 | 货架 | `#shelf` | Hub 云端目录，3 列紧凑卡片（价格/评分/标签/免费额度），次级界面 |
| 3 | 仪表盘 | `#dashboard` | 4 合 1 统计条、SVG 双线趋势图（请求数 kiwi 实线 + token 蓝虚线）、Provider 占比条、Agent 明细表 |
| 4 | 设置 | `#settings` | 通用 / 本地网关（端口、开机启动）/ 隐私（承诺卡片、上报默认关）/ Hub 与导入（CC Switch 配置导入） |
| 5 | 添加供应商弹窗 | — | 480px modal，见 §4 |

## 3. 设计 Token

```css
:root {
  --bg:      oklch(0.15 0.008 260);   /* 深中性底 */
  --panel:   oklch(0.19 0.01 260);
  --border:  oklch(0.28 0.012 260);
  --fg:      oklch(0.93 0.005 260);
  --muted:   oklch(0.62 0.01 260);
  --kiwi:      oklch(0.80 0.19 132);         /* 品牌绿 */
  --kiwi-soft: oklch(0.80 0.19 132 / 0.13);  /* current 行底/软背景 */
  --kiwi-dim:  oklch(0.55 0.14 132);         /* 降亮度：边框/焦点/toggle */
  --blue:    oklch(0.72 0.13 245);    /* token 曲线/辅助 */
  --red:     oklch(0.65 0.2 25);
  --radius:  8px;
  --font-sans: Inter, "Noto Sans SC", system-ui;
  --font-mono: "JetBrains Mono", monospace;  /* 端点/数值/延迟 */
}
```

字体层级：行主名 13px/600，正文 12.5px，辅助 11px，数值一律 mono。

## 4. 添加供应商弹窗（参考 cc-switch AddProviderDialog 模式）

```text
┌─ 480px ────────────────────────────────┐
│ 添加供应商                          ✕  │
│ [🏠 从货架: DeepSeek | 自定义]  toggle │
│ 名称          [DeepSeek            ]  │
│ API Key 仅存本机钥匙串                   │
│               [••••••••••    👁     ]  │
│ 请求地址      [https://api.deepseek.com]│
│               [⏱ 测速 312ms]  （inline结果）
│ 默认模型      [deepseek-chat (V3)  ▾]  │
│ 保存后绑定到Agent                        │
│ [✓Claude Code] [✓Codex] [ Gemini CLI ]  │
│ [⚙ 高级配置（超时/重试/请求头）    ▾]  折叠 │
│                     [取消] [保存并启用] │
└────────────────────────────────────────┘
```

- 从货架选型 → 名称/地址/模型自动填充（cc-switch preset grid 的等价物，改为 toggle 而非网格以省纵向空间）
- Key 永不上传的承诺以 label 内联提示（kiwi 色小字）
- 绑定 Agent 为 checkbox 药丸，选中态 kiwi 边框

## 5. 交互与动效

| 元素 | 规格 |
| --- | --- |
| 行 hover | 150ms bg → `--panel` 亮 4% |
| current 行切换 | 200ms [kiwi左条 X-4→0, α0→1] |
| tab 下划线 | 200ms [X位移, ease-out] |
| modal 进场 | 250ms [S0.96→1, α0→1, ease-out] |
| 测速按钮 | 点击后 spinner，结果 inline mono 显示 |
| 状态点 | 正常=kiwi / 备用=蓝 / 未绑定=灰 / 故障=红（8px 圆点+尾标文字） |

## 6. cc-switch 模式借鉴点（实现时对照）

| cc-switch 模式 | Kiwano 采纳方式 |
| --- | --- |
| AddProviderDialog：preset → 表单 → 高级折叠 | 同结构，preset 改为顶部 toggle（省空间） |
| ProviderCard "current" 绿边 | 列表行 kiwi 左条 + 底色 |
| 健康徽章/故障转移优先级 | 状态列徽章（备用#1/备用#2） |
| 1000×650 + Overlay 标题栏 | 完全一致 |
| 顶部 tab 切换 | 完全一致（不用侧边栏） |

## 7. 用量数据口径（token 字段决策，2026-09-07）

**数据层必须区分，展示层分层：**

- 网关逐请求解析 usage 字段入库（OpenAI 兼容 `prompt_tokens/completion_tokens`；Anthropic `input/output/cache_read/cache_creation`），SQLite usage 表建议字段：`ts, provider_id, agent, input, cache_read, cache_creation, output, latency_ms, cost`
- 计费理由：输出单价 ≈ 输入 3-5×；cache read 按输入 1/10 计价且 Claude Code 场景占比大 → 不区分则「估算费用」失真数倍
- **展示分层**：
  - 首屏行：仅总量 `6.2M tokens · 延迟 1.1s`，In/Out/缓存细分放原生 title tooltip
  - 仪表盘：统计条显示 `8.8M 入7.5M/出1.3M`（tooltip 含缓存说明）；明细表可展开 In/Out

## 8. 计费模型与「用量 / 额度」列（2026-09-07）

Provider 增加 `billing` 字段：`plan（订阅套餐）| payg（按量）| unl（不限）`，决定列表第三列形态：

| billing | 列表展示 | 添加表单字段 |
| --- | --- | --- |
| 订阅套餐 | 环形百分比（紫，28px ring）+ `295/460 请求` + `¥49/月 · 09-30 重置` | 每期上限（请求/万tokens/¥）+ 重置周期（月/周/年/不重置） |
| 按量 · 设限额 | 环形百分比（kiwi）+ `¥28.6 / ¥50 限额` + `6.2M tokens · 延迟` | 消费限额（选填） |
| 按量 · 未设限额 | 总花费 + 近 7 日 sparkline 趋势（蓝） | —（限额留空时自动切趋势） |
| 不限 | `31 次 · 0.2M tok` + `入 0.15M · 出 0.05M`（照常统计，只是不计量费用） | 无（仅提示语） |

- 类型徽章：`按量` 蓝 / `订阅` 紫 / `不限` 灰，9.5px chip
- 环形百分比：28px SVG ring，百分比数字居中（8px mono）；阈值（实现时）`<80%` 主题色，`≥80%` amber，`≥95%` red
- 货架卡片价格行尾加同款计费 chip（Kimi 示例改为 ¥49/月套餐）
- 依据：首屏的核心问题是「这期还能用多少」——额度型用圆环百分比回答，计量无上限的用趋势回答；不限型不计费但 token/次数照常本地统计展示

## 9. 架构决策：Agent 默认接管本地网关（2026-09-07；归因方案 09-07 修订）

首屏「切换即时生效」的语义基础。**每个 Agent 的 base_url 一次性改写指向网关单端口 :8317，并写入网关分配的占位 Key**，之后列表切换 = 纯路由变更，配置文件不再动：

| Agent | 指向方式 | 占位 Key（归因用） |
| --- | --- | --- |
| Claude Code | `ANTHROPIC_BASE_URL=http://127.0.0.1:8317` | `kw-ag-claude-<rand>`（x-api-key / Bearer） |
| Codex | `config.toml` model_providers.base_url | `kw-ag-codex-<rand>`（Bearer） |
| Gemini CLI | `GOOGLE_GEMINI_BASE_URL` 环境变量 | `kw-ag-gemini-<rand>`（x-goog-api-key，P1 接管） |

- **归因**：网关按 auth 头占位 Key 查映射 → usage 表 agent 字段；Key 缺失/未知按路径协议回退到主 Agent（MVP：Anthropic 路径=Claude Code、OpenAI 路径=Codex）。修订说明：初版为每 Agent 独立端口，因占位 Key 方案健壮性等价、扩展性更优（新 Agent 免端口规划）而收敛为单端口
- **收益**：全 Agent 热切换；Agent 归因；P1 故障转移有落点；网关可拒绝无凭证的杂散本地请求
- **代价/配套**：网关常驻守护（GUI 关了代理还在）；每 Agent「接管/还原」开关 + 原配置备份（设置页「Agent 接管」区，还原=逃生门）；Aider/Continue 等不接管
- **UI 落点**：设置 → 本地网关 → Agent 接管列表（已接管 kiwi 标签 + toggle + 占位 Key mono 展示）；首屏 footer 文案「Agent 已接管至本地网关，切换仅改路由」；admin 控制端口 :8310 不在 UI 露出
