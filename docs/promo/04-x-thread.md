# 04 — X / Twitter thread

发布时机:与 Show HN 同日或次日。第一条必须带演示 clip(见 `07-demo-script.md`),
无 clip 第一条只有文字时把视频链接放第二帖。tag 各帖不超过 1~2 个,
主 tag 用 `#ClaudeCode`、`#Codex`、`#AIcoding`、`#opensource`。

---

**Post 1(句号收尾,先给画面)**

```
Your Claude Code, Codex and Gemini CLI all point at ONE local port now.

Kiwano runs a local-first gateway on 127.0.0.1 and takes over 22 coding
agents in one click. Keys stay in an owner-only local DB. Nothing leaves your
machine.

30-second demo: https://youtu.be/M0pO79Wlx-s #ClaudeCode
```

**Post 2(策略层)**

```
Failover that actually fails over:

under single, a dead provider = your afternoon. Under any other Kiwano strategy,
the request is replayed against the next candidate — round-robin, time-window,
quota, whatever you picked.

The agent never sees the outage.
```

**Post 3(协议归一,最能出圈的点)**

```
OpenAI-compatible provider feeding Claude Code?

Yes. The gateway normalizes protocols — anthropic ↔ openai in both directions —
and Gemini's native API passes straight through (metered, not translated).

Protocol is the provider's problem, not yours.
```

**Post 4(成本,踩热点)**

```
The part nobody else shows: real per-request cost, split by agent × provider.

Per-day, 7-day, 30-day trends; quota rings; alerts that fire before the month's
slope blows your limit — not after.

AI spend either gets managed or manages you.
```

**Post 5(信任,展示溯源)**

```
Every "trust us" repo should let you check without trusting anyone:

each release ships SHA256SUMS + a signed build-provenance attestation you can
verify against GitHub's transparency log:

gh attestation verify Kiwano_x64.dmg --repo lightconsen/kiwano

No analytics. No telemetry. Import cc-switch in one command.
```

**Post 6(行动号召,收在链接)**

```
Kiwano is open source (GPLv3+), macOS signed + notarized. Server mode runs
headless with a user-level service — no root.

Downloads: kiwano.cc
Repo + issues: github.com/lightconsen/kiwano

I read every reply. What agent config is broken in the wild? (cc-switch
migration: import works today, configs restored when you switch off.)
```

---

## 配套动作

- [ ] quote-reply 给前三条相关推(能自然带上 demo)的评论
- [ ] 有人贴 profile 图问"这是啥"时,用 Post 1 的 30 秒 clip 回复
- [ ] 连续 6 天每条再补一帖变体,不要 6 帖同一天全发完