# 11 — cc-switch 社区调研(给 9/22 帖与 backlog 供料)

用途:farion1231/cc-switch Discussions 的全量爬梳。两个用处:
①9/22 暖场帖之前知道社区在疼什么,姿态和措辞有据可依;
②把社区反复出现的功能想法收进 backlog 候选池。

数据: Discussions 共 **213 条**(Q&A 103 / General 69 / Ideas 26 / Show-and-tell 14 / Polls 1),
2026-09-21 用 REST API 全量拉取(`repos/farion1231/cc-switch/discussions?per_page=100&page=N`)。
规则会变、话题会涨,写帖前想刷新数据,同一条命令重跑即可。

---

## 0. 一句话结论

社区的高频痛苦按热度排:**①协议 400 ②供应商时好时坏 ③信任焦虑 ④配置被清**。
这四样正好是 Kiwano 主页四个卖点(协议归一 / failover / 信任 / takeover 备份还原)的镜像。
9/22 的帖子和 9/24 的 HN 正文都不需要因此改写——有真实素材可接。

---

## 1. 信任是这个社区最大的情绪痛点(对 9/24 最有用)

| #   | 事件                                                                       | 启示                                                        |
| --- | ------------------------------------------------------------------------ | ----------------------------------------------------------- |
| 2182 | 指控 cc-switch 更新后 5 分钟内 codex `auth.json` 密钥被盗用;维护者索要证据,无结论;楼下有人跟「我也被盗了」 | 「密钥放别人管」的恐惧真实存在,且不需要坐实就会传播                |
| 6726 | 微信文《别把你的数据交给cc-switch了》被转载进 Discussions,维护者下场对线(14 条评论),称文中漏洞均为历史问题已修复 | 安全批评会以「转载」形式绕进社区,争论本身成为流量                |
| 4375 | 山寨网站 `cc-switch.cc` 骗下载;作者澄清唯一官网是 `ccswitch.io`                 | 渠道污染是真实风险,官方安装渠道要一直写在显眼处                  |

**对 Kiwano**:attestation(`gh attestation verify`)、owner-only 本地库、无遥测,
正打在这三个帖子的恐惧上——HN 当天 Post 5 的信任帖完全对口。
**红线:9/22 在 cc-switch 社区发帖时绝对不能提这三条**(runbook 姿态红线:不踩对手)。

---

## 2. 协议兼容性灾难 = 最高频抱怨(Kiwano 核心卖点被逐条验证)

一整类 400 错误,根因全是「参数/协议没有归一就透传给不认识它的上游」:

| #    | 症状                                              | 根因                        |
| ---- | ------------------------------------------------ | --------------------------- |
| 2693 | Claude Code v2.1.97+ 发 `thinking: adaptive` → 三方全 400 | 新参数透传(跟帖:DeepSeek 也犯)   |
| 3216 | 切回 DeepSeek 持续 400「content[].thinking must be passed back」 | thinking 块回传不兼容(另:CC v2.1.154 插件本身有坑,降到 2.1.145 以下可绕) |
| 5864 | 400 `'type' must be in ["enabled","disabled","auto"]` | 同类参数不兼容;关闭思考模式可绕但「几乎不能用了」 |
| 5327 | Codex.app 原生工具 schema 为 null → DeepSeek 400 | tool schema 未清洗            |
| 2487 | Codex 连 DeepSeek 自动加错 URL 后缀                  | 端点拼接错误                   |

**backlog 候选:兼容垫层(compat shim)**——按 provider 自动剥离/改写 `adaptive thinking`、
补齐 thinking 回传、清洗 tool schema。上表每一个 400 在带垫层的网关下都不会发生。
这是「协议归一」从口号变成可演示功能的最短路径。

---

## 3. 最值得抄的想法:模型级路由 + 子 agent 分层(#4721)

CCSwitchMulti fork(BigStrongSun/ccswitchmulti,已给上游提 PR #4292/#4294/#4381):

- Codex 一个本地 provider 后面挂多路模型源:官方订阅旗舰模型 + DeepSeek/Qwen/vLLM 混用;
- **按模型名精确/前缀匹配路由**,请求分发到对应上游;
- 模型目录、上下文窗口、能力回填到 Codex 可见的 catalog,避免显示成 `custom`;
- **子 agent 专门适配**:主 agent 留在旗舰模型(规划/审查/决策),子 agent 自动迁到
  便宜模型(改写 `~/.codex/agents/*.toml`);
- 已知差评:有用户反馈用了会丢历史会话。

**对 Kiwano**:现在策略是 provider 级(single/failover/roundrobin/timewindow/quota)。
「按模型名路由」与「子 agent 自动降级到便宜模型」是自然延伸——后者与 insights 的
成本归因(每 agent × 每 provider)天然互补,能直接算出「子 agent 降级省了多少钱」。

---

## 4. 供应商健康探测(#5604 CC-Pulse)

第三方工具 CC-Pulse:真发请求测供应商能不能用,不是只测连通。作者踩过的坑:

- 200 但 body 是业务错误(「旧转发链路已关闭」),没有模型输出;
- 200 但答案为空(thinking 模型预算不足时只「想」不「答」);
- key 已废,能列模型不能聊天;
- **请求 A 模型,响应里静默变成 B 模型**;
- 有的站按 User-Agent 封 Claude Code 的 UA(403)。

检测方法值得抄:haiku/sonnet/opus 多档回退 + 发 `2+3=?` 且答案必须是 5 才算过 +
流式/tool use/thinking 单项深检。

**对 Kiwano**:验证 failover 的真实需求(「看着是绿的,切过去就挂」)。
**backlog 候选:内置「供应商体检」**——对单个 provider 发冒烟请求,报告逐项可用性。

---

## 5. 缓存命中率调优(#6172,含金量高)

DeepSeek 走 OpenCode Go:OpenAI Chat Completions 格式下缓存命中长期卡 **89.5%**;
改成 **Anthropic Messages 原生格式 + `ANTHROPIC_API_KEY`**(注意 AUTH_TOKEN 会 401)
后升到 **95–96%** 且继续上升。原理:缓存前缀稳定性依赖协议形态。
(后续跟帖:该路径后来也开始报 tools[0].function 反序列化 400——中转质量本身不稳。)

**对 Kiwano**:直接验证 insights 的 cache hit 指标价值。
**backlog 候选:格式选择建议**——能走原生 `/v1/messages` 就别走 chat/completions,
在 provider 编辑页给出提示(该案例还顺带证明:供应商文档说「只支持 OpenAI 格式」
不一定是真的,实测才算数)。

---

## 6. 其余值得记一笔的(按 Kiwano 相关度排序)

| #        | 想法/事件                                                        | Kiwano 现状 / 动作                    |
| -------- | ---------------------------------------------------------------- | ------------------------------------ |
| 3734/5872 | **per-provider 出站代理**(全局代理不够:A 走公司代理 B 直连;有人要 3 份配置各挂一个代理) | 需确认 Kiwano 是否支持;没有就进 backlog。中国用户刚需 |
| 6548     | 切回官方后**原配置被清空**且无处找回                               | Kiwano takeover 已做备份+还原 ✓(9/22 可提的点) |
| 4437     | SSH 远程 vibe coding 时想**用 CLI 切换**                            | Kiwano server + CLI 已覆盖 ✓(9/22 可提的点) |
| 1536     | 想要 cc-switch 自身的 **MCP**(外部自动化与本地 agent 交互)          | Kiwano insights 已有 MCP 查询 ✓       |
| 2171     | **离线估算配额**:断网/5xx 时用本地 session 日志估 5h/7d 窗口,别让面板空白 | 可借鉴:quota 环的离线兜底逻辑         |
| 4933     | 成本想要**人民币单位**                                             | Kiwano 原生币种记账 ✓(显示货币规则见既有约定) |
| 1166     | iTerm2 状态栏**只读 cc-switch SQLite** 显示用量                     | 生态信号:只读 DB 的外部工具形态受欢迎,Kiwano 的 SQLite 同样可以长出这类周边 |
| 7329     | p2p 给朋友临时共享 plan 额度(cc-switch-remote fork)                | 有趣但有条款风险,只记录不跟进         |
| 2913/2063/4956/4579 | 求更多工具支持:Cursor、Kiro-CLI、Antigravity、qoder     | 注意评论:Kiro/Antigravity 不支持 BYOK。另有说法称 Google 将弃更 Gemini CLI 主推 Antigravity——agent 生态变化值得跟踪 |
| 3262     | mimo token plan 需要独立端点(CN/AMS 节点分付)                      | 验证 Hub 货架按 plan 分条目的设计 ✓   |
| 3413     | 求 CCMimoLink 内置预设                                             | 同上,货架类需求                       |
| 4435/4410/4828/2515 | 主界面项目启动器;跳过模型清单检查开关;路由入口不统一(Claude Desktop 一级菜单 vs Claude 在设置里);首选终端 | UX 细节类,Kiwano 自查时参考           |
| 1493     | 求低版本 glibc(RHEL8)可用的 release                              | Kiwano 已处理过 glibc floor(见 CHANGELOG/README per-distribution 说明)✓ |
| 5604 跟帖/5327 跟帖 | 有人借 bug 帖推竞品(deepnow:「比 cc-switch 好的多」)     | 竞品在借客服场景获客;不接茬,做好自己的体检能力 |

---

## 7. 用法

- **9/22 发帖前**:重读 §1 的红线与 §6 里三个 ✓ 点(配置还原、CLI、MCP)——
  它们是 cc-switch 用户当场能验证的差异,也是 `06-cc-switch.md` 正文的既有内容,别超纲。
- **backlog**:§2 兼容垫层、§3 模型级路由+子 agent 分层、§4 供应商体检、§5 格式建议、
  §6 per-provider 代理——五个候选,做完任何一个都可以按 runbook 阶段 6
  「每个小版本挑一个亮点单独发一帖」。
- **9/24 HN**:正文不改;评论区若有人问「和 cc-switch 区别」,§1–§5 就是「往前走一步」
  的具体答案(口径仍走 `01-pitch.md` 的对比表)。
