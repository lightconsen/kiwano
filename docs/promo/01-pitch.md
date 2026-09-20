# 01 — 定位与事实源

用途:所有渠道材料(02~07)从本文件取事实与措辞,避免各帖说法不一致。
事实一律以 `README.md` / `docs/cli.md` / `CHANGELOG.md` / `THIRD-PARTY-NOTICES` 为准。

## 一句话定位(default)

> **A local-first AI provider manager.** Keep every provider key on your own
> machine, point all your coding agents at one local gateway, and see what they
> actually cost.

(中译:「本地优先的 AI Provider 管理工具。Key 留在本机,所有编码 Agent 走一个本地
网关,并看到它们真实花了多少钱。」)

## 电梯版(30 秒口述)

Kiwano 是桌面 App,装好后在 `127.0.0.1:8317` 起一个本地网关。你注册一次 Provider,
就可以一键把 Claude Code、Codex、Gemini CLI、Cline 等十几个 Agent 全部"接管"到这一个
端口上。网关负责:协议归一(OpenAI Provider 也能喂给 Claude Code)、按策略路由
(single / failover / roundrobin / timewindow / quota,非 single 策略下失败的请求自动
换下一候选重放)、计量每次请求的花费并记日志。Key 存在只有你能读的本地 SQLite 里,
请求不经过任何 Kiwano 服务器。同一套也能跑在服务器上:`curl -fsSL https://hub.kiwano.cc/install.sh | sh`。

## 三张可打的差异牌(每渠道挑一张,别全打)

1. **本地优先 / 隐私** — 对比托管网关(LiteLLM/自建 proxy):请求与 Key 不出本机,
   "zero telemetry"。资源最像 2010 年代的"own your data"叙事。
2. **成本计量** — AI 花费管理是当下 B 端最热的痛点:按 provider / 按 agent 归因,
   今日 / 7 日 / 30 日趋势、quota ring、成本告警甚至月内预测。
3. **多 Agent 单端口 + 故障转移** — "One port, every agent." 一个 Agent 挂了换下一个,
   请求不被浪费。

## 事实核对面(发帖前逐条对 README)

- 端口:`127.0.0.1:8317`。
- 接管 Agent 列表(以 README 为准,截至 0.1.16):
  Claude Code、Codex、Gemini CLI、Grok Build、Claude Desktop、OpenCode、OpenClaw、
  Hermes、Pi、WorkBuddy、CodeBuddy Code、Kimi Code CLI、Qwen Code、Cline。
  接管时原配置备份,关闭时恢复。
- 协议归一:anthropic / openai 双向;Gemini 原生 API 作为第三种协议直通放行(只计量不翻译)。
- 路由策略:single / failover / roundrobin / timewindow / quota。
- Models shelf:Hub 上的 **24 个 Provider**(23 个官方 + 1 个聚合 OpenRouter),
  离线可用,条件同步(manifest hash 跳过未变更)。
  > 已实测:`curl -s https://hub.kiwano.cc/catalog.json` → `total = 24`,
  > `manifest.json` 的 `catalog.count = 24`(2026-09-20)。**README 里写的 19 是过期数字,已一并修正。**
  > 发帖前若目录又变了,以线上 `catalog.json` 的 `total` 为准,别背数字。
- 隐私:Key 在 owner-only SQLite(目录 0700 / 文件 0600);请求与正文不过 Kiwano
  服务器;无遥测无埋点;更新检查只是对静态 JSON manifest 的一次 HTTPS 请求。
- 服务器形态:一键 `curl -fsSL https://hub.kiwano.cc/install.sh | sh`,装 CLI + 网关
  daemon 到 `~/.local/bin`,注册用户级服务(免 root)。CLI 能力见 `docs/cli.md`
  (`kiwano import cc-switch`、`kiwano agents takeover`、`kiwano insights`、
  `kiwano mcp`、`kiwano rules apply|remove|status`、`kiwano logs show`…)。
- 演示视频:**https://youtu.be/M0pO79Wlx-s**(30 秒,2026-09-20 上传 YouTube)。
  各渠道正文统一用这个短链(Show HN / X / Reddit / V2EX / cc-switch 帖均已填入),
  落地页首屏也用它。
- 下载:`https://github.com/lightconsen/kiwano/releases/latest`(GitHub 镜像)与
  `https://hub.kiwano.cc/releases/`(R2 永久链接,除 Download 表外不写版本号)。
  macOS 已用 Developer ID 签名并公证;Windows 未签名。
- 信任基建(别人少有的加分项):每个发布附 `SHA256SUMS` + 可验证构建溯源
  (`gh attestation verify <file> --repo lightconsen/kiwano`,走 GitHub 透明度日志)。
- 许可证:GPLv3-or-later;部分模块移植自
  [cc-switch](https://github.com/farion1231/cc-switch)(MIT,见 `THIRD-PARTY-NOTICES`)。
- 平台:macOS(Apple Silicon / Intel)、Windows 10+、Linux x86_64。

## 对比表(给 README / 评论区复用)

| | Kiwano | cc-switch | LiteLLM / 自建托管网关 |
| --- | --- | --- | --- |
| 形态 | 桌面 App + 本地网关 + CLI | 只切换 Claude Code 配置 | 服务器常驻服务 |
| 覆盖 Agent | 14 个,一键接管+还原 | 主要是 Claude Code 生态 | 仅接入了配置的客户端 |
| 本地优先 | 全程本机,零遥测 | 本机 | Key 在服务器 |
| 路由策略 | 5 种 + 自动重放 | 切换,不路由 | 视实现 |
| 计量/分析 | 按 provider×agent 归因 + 告警 | 无 | 通常有 |
| 现状 | 持续迭代 | 成熟 | 重型 |

一句话立场:**cc-switch 解决"换 Provider",Kiwano 解决"管全部 Provider 和全部
Agent、并知道花了多少钱"。** 前者是后者的子集(还提供 import 迁移),别踩前者,
姿态要"向前走一步"而非"替代品"。

## FAQ(评论区/工单预演)

- **和 claude-code-router 比?** 我们聚焦 14 个 Agent 而不只 Claude Code,并且把
  计量、告警、desktop 管理做成一体;ccr 是很有趣的 Claude Code 专用方案。见对比表。
- **Key 安全吗?** 本地 SQLite,目录 0700/文件 0600;凭证只发给配置的 Provider;
  网关 /metrics 默认哈希 agent 标签,需要时可加 Bearer token 全名鉴权。
- **免费吗?** GPLv3-or-later 开源,Hub 是元数据目录(价格/名称/端点),不碰数据。
- **Windows 签名?** 还没签,SmartScreen 会提示,「更多信息 → 仍要运行」;这是我们
  已知短板,正在处理。
- **为什么选 Rust/Tauri?** 网关 daemon 与 CLI 需要无 GUI 也能跑(服务器模式),后端
  与前端同仓同 crate(`kiwano-core` 共享业务层)。