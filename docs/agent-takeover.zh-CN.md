# Agent 接管

接管是让 Agent 走本地网关的方式:Kiwano 改写 Agent **自己**在其 `$HOME`
下的配置文件,改写前先备份,关闭接管时按字节还原。不注入代理二进制,不替
你导出环境变量——Agent 照常读自己的配置,发现网关就在那里。

本页是按 Agent 的参考:每个接管写哪些文件、用什么模式、哪些行为值得知道。
Agent 集合与这些路径由一致性测试钉住(docs 表格 ↔ 内置注册表),本页不会
与某个构建的行为悄悄漂移。

## 三种模式

- **Additive(增量)**——Agent 配置里是供应商列表;接管 upsert 一条
  `kiwano-gateway` 条目并选中它。你自己的供应商原样保留。
- **Exclusive(独占)**——配置就是一个供应商槽位;接管直接改写。还原时
  把你的字节放回去。
- **替换选中槽位**(仅 Cline)——其选择器指向一个供应商 id,接管改写的
  是该 id 指向的槽位,而不是在旁边加第二条。

## Agent 一览

| Agent | 被改写的配置 | 模式 | 值得知道 |
| --- | --- | --- | --- |
| `claude` | `~/.claude/settings.json` | 独占 | `CLAUDE_CONFIG_DIR` 会搬动整个 profile。 |
| `codex` | `~/.codex/config.toml` + `auth.json` | 独占 | 认 `CODEX_HOME`。两文件一起改写;`config.toml` 缺失时拒绝——先跑一次 Codex。 |
| `gemini` | `~/.gemini/.env` | 独占 | 文件缺失会创建;其中的注释保留。 |
| `grokbuild` | `~/.grok/config.toml` | 独占 | 拒绝官方 xAI 登录的配置——先配好自定义模型。 |
| `claude-desktop` | 两份 `claude_desktop_config.json` + configLibrary profile + `_meta.json` | 独占 | 仅 macOS;把部署模式切到 3p 并写入网关 profile。 |
| `opencode` | `~/.config/opencode/opencode.json` | 增量 | 走 XDG 规则(`XDG_CONFIG_HOME`)。 |
| `openclaw` | `~/.openclaw/openclaw.json` + `agents/<id>/agent/models.json` | 增量 | 目录文件承载主配置校验器不接受的会话亲和标志。 |
| `hermes` | `~/.hermes/config.yaml` | 增量 | 认 `HERMES_HOME`。 |
| `pi` | `~/.pi/agent/models.json` + `settings.json` | 增量 | 选择写在 settings 里。 |
| `workbuddy` | `~/.workbuddy/models.json` | 增量 | 认 `WORKBUDDY_CONFIG_DIR`。 |
| `codebuddy` | `~/.codebuddy/models.json` | 增量 | 认 `CODEBUDDY_CONFIG_DIR`。 |
| `kimi` | `~/.kimi-code/config.toml` | 增量 | 认 `KIMI_CODE_HOME`;后继版缺席时读旧版 `~/.kimi`。 |
| `qwen` | `~/.qwen/settings.json` | 增量 | 认 `QWEN_HOME`。 |
| `cline` | `~/.cline/data/settings/providers.json` | 替换选中槽位 | 依次认 `CLINE_PROVIDER_SETTINGS_PATH` / `CLINE_DATA_DIR` / `CLINE_DIR`。 |
| `mimo` | `~/.config/mimocode/mimocode.jsonc` | 增量 | 走 XDG 规则。 |
| `mcode` | `~/.minimax/config.yaml` | 增量 | 认 `MCODE_CONFIG_DIR`。 |
| `aider` | `~/.aider.conf.yml` | 独占 | 三个全局字段,不是供应商列表。模型加 `openai/` 前缀强制走 LiteLLM 的 OpenAI 兼容路由;模型 id 本身原样到达上游。`AIDER_CONFIG` 刻意不认。 |
| `continue` | `~/.continue/config.yaml` | 增量 | 网关模型插到 `models` 头部并限定 `roles: [chat]`——Continue 的文档化默认是"该角色的第一个模型"。 |
| `crush` | `~/.config/crush/crush.json` | 增量 | 填它自己 `model large` 命令持久化的 `models.large` 槽;`small` 槽不动。 |
| `droid` | `~/.factory/settings.json` | 增量 | 在 `customModels` 头部插入条目并设默认 `model`。组织策略 `allowCustomModels: false` 时拒绝接管。 |
| `goose` | `<goose 配置根>/config.yaml` + `custom_providers/kiwano-gateway.json` + `kiwano-gateway.key` | 增量 | macOS 根目录是 `~/Library/Application Support/Block/goose`(不是 `~/.config/goose`);Linux 走 XDG。key 通过供应商的 `auth.command`(`/bin/cat` key 文件)送达,因为 goose 的 keyring 秘密存储不读文件。首次导入:无——key 在 keyring 里。 |
| `zcode` | `~/.zcode/v2/provider_config.json` | 增量 | ZCode CLI 与 Desktop 共享的原生注册表——接管同时影响两端。认 `ZCODE_PERSONAL_PROVIDER_CONFIG_FILE`;不认 `ZCODE_DATA_BASE_DIR`。 |

## 环境变量

接管通过用户的**登录 shell**环境解析路径(GUI 应用本来也看不见 rc 文件
导出的东西)。变量被设成*相对*路径时直接拒绝,而不是猜。

认: `CLAUDE_CONFIG_DIR`、`CODEX_HOME`、`XDG_CONFIG_HOME`(opencode、
mimo、crush;Linux 上的 goose)、`HERMES_HOME`、`WORKBUDDY_CONFIG_DIR`、
`CODEBUDDY_CONFIG_DIR`、`QWEN_HOME`、`KIMI_CODE_HOME`(及旧版
`KIMI_SHARE_DIR`)、`CLINE_PROVIDER_SETTINGS_PATH`、`CLINE_DATA_DIR`、
`CLINE_DIR`、`MCODE_CONFIG_DIR`、`ZCODE_PERSONAL_PROVIDER_CONFIG_FILE`。

刻意不认: `AIDER_CONFIG`(aider v1.24 加载时不读它)、`GOOSE_PATH_ROOT`
和 `ZCODE_DATA_BASE_DIR`(搬整棵树会把接管要一起写的文件拆散)、
`OPENCODE_CONFIG` / `OPENCODE_CONFIG_DIR` 和 OpenClaw 的状态覆盖(它们
不会搬走接管要写的那个文件)。

## 首次接管的导入

Agent 已配置过自定义端点时,开启接管会提议导入:同一 base URL 和 key 变成
一条绑定到该 Agent 的供应商,第一天路由的还是同一个上游。导入永不把
loopback 端点当成上游(那是接管后的网关自己),也读不出不存在的明文
key——官方订阅登录(Claude OAuth、Codex ChatGPT)和 goose 的 keyring 什么
都取不出来,引导改为手动录入。

## 还原语义

关闭接管:备份的原件按字节写回(接管创建的文件被删除)。备份丢失——或
备份是在配置本已指向我们时抓的——则用网关为该 Agent 服务的供应商重建
路由;再不行,就把我们的路由从活配置里**剥离**,保证 Agent 不会被留在
指向无人监听的 loopback 端口。剥离是外科手术式的:只删属于我们的
行/值。
