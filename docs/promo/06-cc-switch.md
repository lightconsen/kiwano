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
127.0.0.1:8317 that takes over 14 coding agents (Claude Code, Codex, Gemini
CLI, Cline, ...) with one click, routes request-by-request across providers by
strategy (failover / roundrobin / timewindow / quota), and meters real
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

## 给操作者的中文摘要

发帖时要说的意思就三点:①引用过你们 MIT 的模块、署名在案;②我们的视角是"再往前走
一步"——从"切换"到"全部托管 + 计量",你现有的配置 `kiwano import cc-switch` 一键迁;
③问他们:还缺什么 Provider 记录/Agent 配置要导入的,我们照单改。语气是"同胞项目",
不是踢馆。

## 其他可顺带的 cc-switch 触点

- 它的主页 README 若接受"生态"链接,可提议加一条入口(礼貌、只提一次)。
- 中文圈 cc-switch 的讨论多在博客/公众号,用 `05-v2ex.md` 的帖子接住。
- 如果有人问"为什么不用 xxx",回答口径永远在 `01-pitch.md` 的对比表里。