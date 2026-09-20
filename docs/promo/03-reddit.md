# 03 — Reddit 帖子模板

先决条件(失败即掉进广告位):

- 在目标 sub 有**参与历史**(评论 ≥ 若干条、有 karma),新号直接发"我做了个工具"会被
  自动规则挡住或判 spam。
- 标题**不带链接**;正文开头先自报"oc,open source"。
- 发帖节奏与 Show HN 错开 2~3 天。

---

## A. r/ClaudeAI —— 主帖(体量最大)

标题(≤300 字符,不带链接):

```
I'm building Kiwano, a local gateway so all my agent configs share one port — failover, quotas and per-request cost, keys never leave the machine
```

正文:

```
I got tired of juggling `~/.claude/settings.json` vs Codex vs Gemini CLI, and of
never knowing what a session actually cost. So I wrote Kiwano: a desktop app
that runs one local gateway on 127.0.0.1:8317 and takes over 14 agents (Claude
Code, Codex, Gemini CLI, Grok Build, Cline, Kimi/Qwen Code, Claude Desktop, …)
in one click — originals backed up, restored when you switch off.

What it does that I couldn't get elsewhere:

- Protocol normalization — an OpenAI-compatible provider can serve Claude Code.
- Routing strategies (single / failover / roundrobin / timewindow / quota);
  outside `single`, a failed request is replayed against the next candidate.
- Real metering: requests/tokens/cost per provider × per agent, 24h/7d/30d,
  quota rings + cost alerts (even a forecast before the month's slope passes
  your limit).
- Models shelf with 24 providers (official first-party + OpenRouter).
- Import from cc-switch, so migration is one command: `kiwano import cc-switch`.

Privacy matters to me here: keys live in an owner-only local SQLite (0700/0600),
no telemetry, no analytics. Every release carries SHA256SUMS and a signed
build-provenance attestation (verifiable via `gh attestation verify`).

Open source, GPLv3+. macOS signed+notarized; Windows not yet signed (known gap).

30-second demo: https://youtu.be/M0pO79Wlx-s
Downloads: https://kiwano.cc
Repo: https://github.com/lightconsen/kiwano

Honestly interested in what's missing — which agent config is broken in the
wild, and what your cost panel isn't telling you.
```

---

## B. r/ChatGPTCoding —— 变体(更偏"开发者在用")

标题(英文,直接复制):

```
Show and tell: a local gateway that routes Claude Code, Codex and Cline through every provider I hold — with per-request cost metering
```

正文改动(**中文说明,不要复制进帖子**;正文主体沿用 A 节,按下面四处替换):

- 开头换成:"I spend ~$400/mo across providers and never knew the split. So I
  built a local gateway that meters every request per agent per provider."
- 条目标题换成 `What problem it actually solves:`
  1. One port for every agent (14 agents takeover, originals restored)
  2. Failed requests replay against the next candidate under any strategy but `single`
  3. Cost split by agent and provider, with a forecast before you blow the budget
  4. 24 providers in a sync-once, work-offline models shelf
- 保留隐私段、信任段、链接;结尾去掉"what's missing"改成"happy to answer adoption
  questions"。

> 注:这一节是"改哪里"的清单,**不是成稿**。A 节是唯一可直接整段复制的正文。

---

## C. r/selfhosted —— 服务器视角(可选项)

这个社区**反感"桌面 App 来蹭榜单"**,所以第一段必须是 headless 网关,桌面 App 放到最后
一句带过。先确认已读过该 sub 的自荐规则(多数要求作者本人发、且不带返利链接)。

标题(英文,直接复制):

```
A self-hostable AI gateway for coding agents: one-line install, user-level service, Prometheus metrics, per-agent cost metering
```

正文:

```
I run several coding agents on a headless box and got tired of each one holding
its own provider keys, with no idea what any of them cost. So I built Kiwano.

It is a local-first AI gateway. On a server it is just a daemon plus a CLI:

    curl -fsSL https://hub.kiwano.cc/install.sh | sh

That installs both binaries to ~/.local/bin and registers the gateway as a
service for your own user — no root, no Docker, no display. It is a plain shell
script and it is in the repo if you want to read it before running it.

What it does:

- One port (127.0.0.1:8317) for every agent. Agents are taken over by rewriting
  their own config under their own $HOME, with the original backed up and
  restored when you switch the takeover off. That is also why the service must
  run as the same user as the agent.
- Routing strategies per agent: single / failover / roundrobin / timewindow /
  quota. Outside single, a failed request is replayed against the next candidate
  instead of being handed back to the agent.
- Protocol normalization, so an OpenAI-compatible provider can serve an
  Anthropic-protocol agent. Gemini's native API is passed through rather than
  translated — metered, not rewritten.
- Cost metering per agent × per provider, with quota alerts.
- GET /health for a supervisor and GET /metrics in Prometheus text format.
  /metrics is open to local processes by default with agent labels hashed; run
  the daemon with KIWANO_METRICS_TOKEN set and it requires a bearer token and
  serves the full names.

Keys live in an owner-only local SQLite (dir 0700, file 0600) and are only ever
sent to the provider you configured. No telemetry, no analytics, no account.

There is also a desktop app (Tauri) over the same core crate — same rows, same
writes — but the server mode above is what I would want reviewed here.

GPLv3+. Releases carry SHA256SUMS and a signed build-provenance attestation:

    gh attestation verify kiwano-x86_64-unknown-linux-gnu.tar.gz --repo lightconsen/kiwano

Repo: https://github.com/lightconsen/kiwano
Docs for the CLI: docs/cli.md in the repo.

Happy to hear what is missing for a real deployment — auth model, metrics worth
scraping, whatever your setup needs.
```

---

## 通用发布后动作

- [ ] 帖子发出后 30 分钟内不回,看风向;回帖只加实质信息
- [ ] 每个 sub 发一次即可,**不要改链接再重发**
- [ ] 有人搬运/讨论时,把 FAQ(`01-pitch.md`)里的对比答案贴过去