# 快速上手

Kiwano 位于你的编码 Agent 与 AI 供应商之间:供应商注册一次,本地网关跑在
`127.0.0.1`,所有 Agent 都指向这一个端口。本页从安装带到第一条被计量的
请求。Key 永远不离开你的机器。

## 你需要准备

- 一个供应商 API Key(OpenAI 兼容或 Anthropic 兼容端点)。
- 一个或多个已安装的编码 Agent——Claude Code、Codex、Gemini CLI……
  (全部 22 个内置 Agent 见 [Agent 接管](agent-takeover.zh-CN.md))。

## 1. 安装并启动

从 [Releases](https://github.com/lightconsen/kiwano/releases/latest) 下载
对应平台的签名安装包(macOS / Windows / Linux)。首次启动会拉起 **守护进程**
——本地网关,它的生命周期独立于 GUI 窗口:关掉 App,网关仍在服务。托盘图标
显示其状态。

## 2. 注册供应商

在 **Models** 货架(Hub 目录)里搜索供应商——DeepSeek、Kimi、智谱 GLM、
OpenRouter……——或在 **Apps** 屏手动添加:

![Models 货架:Hub 目录,按类别归档,含协议、计费与价格](screenshots/shelf.webp)

*Models 货架——Hub 目录:官方、聚合、第三方、免费四类供应商,计费与价格透明。*

1. **Add provider** → 名称、端点、API Key、端点使用的协议(`openai` 或
   `anthropic`)。
2. 可选:默认模型、计费类型(按量 / 订阅 / 不限量)和花费限额。
3. 保存。一旦有请求打到它,供应商行的健康状态就会出现。

Key 保存在仅限当前用户读取的本地数据库里。除了你填写的端点,数据不发往
任何地方。

## 3. 接管一个 Agent

**Apps** 屏上每个受支持的 Agent 都是一张卡片:

![Apps 屏:供应商列表,带绑定的 Agent、7 天用量与配额、状态](screenshots/apps.webp)

*Apps 屏——供应商与绑定的 Agent 一目了然;用量、配额与健康状态就在行上。*

- 如果该 Agent 已配置过供应商,**Enable Kiwano** 会提议导入它——上游在
  第一天就保持不变,只是改为经过网关(从而被计量)。
- 接管开关会改写 Agent 自己的配置文件,把端点指向网关。改写前先备份,
  关闭接管时按字节还原。Agent 的其它配置一概不动。

每个 Agent 具体改写哪些文件、哪些行为特殊,见
[Agent 接管](agent-takeover.zh-CN.md)的表。

## 4. 验证

通过 Agent 发一条 prompt(`claude "hi"` 或日常用法),然后打开 Kiwano 的
**Request logs**:应能看到这条请求,带归属(哪个供应商服务的)、token、
延迟和估算成本。Dashboard 会把同样的行按供应商、按 Agent、按天聚合。

![Dashboard:统计块、用量趋势、按供应商与 Agent 归因、请求日志](screenshots/costs.webp)

*Dashboard——7 天趋势按供应商与 Agent 归因;下方日志行可展开单次请求。*

## 5. 一个 Agent 路由多个供应商

一个 Agent 绑一个供应商是 `single` 策略。给 Agent 的路由多绑几个候选,
再选一个策略——`failover`、`roundrobin`、`timewindow`、`quota` 或
`least-busy`——并设置按 Agent 的花费限额。见[策略与限额](strategies.zh-CN.md)。

## 数据都在哪

| 内容 | 位置 |
| --- | --- |
| 供应商、路由、用量、备份 | 本地 SQLite(仅当前用户可读),由守护进程读写 |
| Agent 配置 | 各 Agent 自己在 `$HOME` 下的文件——见接管表 |
| 请求日志与报文 | 本地,保留期在设置里可调 |

## 卸载 / 切换

按 Agent 关闭接管(配置从备份还原),或删除供应商(自动提升下一个候选)。
守护进程可从托盘停止;不会有 Agent 被留在指向死端口的状态——被移除的接管
会还原,备份丢失的接管会降级回 Agent 自己的默认配置,而不是一个没人监听的
loopback 地址。
