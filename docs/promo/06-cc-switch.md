# 06 — cc-switch 社区迁移帖(outreach)

目标:在 [farion1231/cc-switch](https://github.com/farion1231/cc-switch) 的
Discussions 发一条。这是最先做的一条冷启动——受众就是"已经在管理多个 Provider"的
存量用户,而且你开源引用过它的代码,话题天然成立。

## 姿态红线(比内容更重要)

1. **绝不自称"替代品"**。cc-switch 管"切换",Kiwano 管"全部 Provider + 全部 Agent
   + 计量"。Kiwano 的 `import cc-switch` 是迁移入口,不是踩踏。
2. **先致谢**。第一段就点名 MIT 移植了它的 agent 配置适配层,署名在
   `THIRD-PARTY-NOTICES`,把帖子变成"同胞项目"而非"竞品挑衅"。
3. 只发一条、只在一个 Discussion 里发;回贴要诚恳,被删就算了,不纠缠。

## Discussion 正文(英文,可直接粘贴)

```
Title: kw: a local-first gateway that imports cc-switch configs | migration path

Hi cc-switch folks — quick note from the maintainer of Kiwano, an open-source
project that was built *on top of* cc-switch's approach (selected agent-config
adapter modules are ported from this repo, MIT, credited in THIRD-PARTY-NOTICES).

Kiwano is a local-first AI provider manager: a desktop app + local gateway on
127.0.0.1 that takes over 22 coding agents (Claude Code, Codex, Gemini
CLI, Cline, ...) with one click, routes request-by-request across providers by
strategy (failover / roundrobin / timewindow / quota / least-busy), and meters real
per-request cost per agent × provider.

If you are on cc-switch today you can migrate your existing config in one
command:

    kiwano import cc-switch

It reads the current cc-switch config and brings providers over; takeover
backups each agent's original config and restores it when you switch the
feature off. Keys stay in an owner-only local SQLite, no telemetry, and every
release ships SHA256SUMS + a signed provenance attestation.

Downloads: https://kiwano.cc — 30s demo: https://youtu.be/M0pO79Wlx-s — repo: https://github.com/lightconsen/kiwano

Totally open about where this stands relative to you: cc-switch solves
"switching providers". Kiwano is a step forward toward "managing every provider
AND every agent, and knowing what it all costs" — switching is folded in, and
migration is explicit. Happy to hear what I should import next (which provider
records, which agent configs) so the path in is painless. Thanks forever for the
MIT code this project stands on.
```

## Discussion 正文(中文版,可直接粘贴)

> cc-switch 用户群以中文为主,**默认发这一版**;英文版留作楼中楼或备选。

```
标题:Kiwano:一个本地优先的网关,一键导入 cc-switch 配置 | 迁移路径

大家好——我是 Kiwano 的维护者。这个开源项目是站在 cc-switch 的肩膀上做的:
其中一部分 agent 配置适配模块移植自本仓库(MIT 协议,已在 THIRD-PARTY-NOTICES
中署名致谢)。

Kiwano 是一个本地优先的 AI Provider 管理器:桌面应用 + 跑在本机 127.0.0.1 的
网关,一键接管 22 个编码 agent(Claude Code、Codex、Gemini CLI、Cline……),
按策略(failover / roundrobin / timewindow / quota / least-busy)逐请求在多个 Provider 之间
路由,并按 agent × provider 计量每一笔真实成本。

如果你现在在用 cc-switch,一条命令即可迁移现有配置:

    kiwano import cc-switch

它会读取当前 cc-switch 配置并迁入 providers;接管功能会备份每个 agent 的原始
配置,关闭接管时自动恢复原状。密钥只存在 owner-only 的本地 SQLite 里,无遥测,
每个 release 附 SHA256SUMS 与签名构建溯源。

下载:https://kiwano.cc —— 30 秒演示:https://youtu.be/M0pO79Wlx-s ——
仓库:https://github.com/lightconsen/kiwano

坦白说我们和你们的位置关系:cc-switch 解决的是"切换 Provider";Kiwano 往前多走
了一步——"托管全部 Provider 和全部 Agent,并且知道这一切花了多少钱",切换被
折叠了进来,迁移路径是显式的。欢迎告诉我还缺什么(哪些 Provider 记录、哪些
agent 配置需要导入),我们把迁入路径做顺。再次感谢本项目所依赖的 MIT 代码。
```

## 给操作者的中文摘要

发帖时要说的意思就三点:①引用过你们 MIT 的模块、署名在案;②我们的视角是"再往前走
一步"——从"切换"到"全部托管 + 计量",你现有的配置 `kiwano import cc-switch` 一键迁;
③问他们:还缺什么 Provider 记录/Agent 配置要导入的,我们照单改。语气是"同胞项目",
不是踢馆。

## 其他可顺带的 cc-switch 触点

- 它的主页 README 若接受"生态"链接,可提议加一条入口(礼貌、只提一次)。
- 中文圈 cc-switch 的讨论多在博客/公众号,用 `05-v2ex.md` 的帖子接住。
- 如果有人问"为什么不用 xxx",回答口径永远在 `01-pitch.md` 的对比表里。