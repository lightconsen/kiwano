# 02 — Show HN 帖子

用途:发布到 <https://news.ycombinator.com>。HN 标题上限 **80 字符**,介绍文宜短、
附视频。发布时段:周二~周四,UTC 14:00–17:00(美东早 10~13 点)。

## 标题(选一)

HN 标题字段上限 **80 字符**,超了直接提交失败。下面三条已实测长度(含 `Show HN: ` 前缀):

| # | 长度 | 标题 |
| --- | --- | --- |
| 1 | 73 | `Show HN: Kiwano - one local port, every agent, every provider, real costs` |
| 2 | 69 | `Show HN: Kiwano - a local gateway for every coding agent and provider` |
| 3 | 77 | `Show HN: Kiwano - keep every AI key local, one gateway, real cost per request` |

选第 1 个为默认(最能一句话说清差异化)。发布后 2 小时内别改标题(改标题会重新计分排名);
若想换,趁早换一次。

> 反例(不要用,均已超限):`…local AI gateway for every coding agent, with failover and cost metering`
> 是 90 字符;`…local-first provider manager so Claude Code, Codex and Gemini CLI all share one gateway`
> 是 105 字符。起草新标题时先数长度。

## 正文(可直接粘贴;`[…]` 处换成最终链接)

```
Kiwano is a local-first AI provider manager: register providers once, and it
runs a local gateway on 127.0.0.1 that every coding agent — Claude Code,
Codex, Gemini CLI, Grok Build, Cline and 9 more — talks to through one port.

https://kiwano.cc
https://github.com/lightconsen/kiwano
(both the repo and the landing page are English-first, with a Chinese version)

The gateway normalizes protocols (so an OpenAI-compatible provider can serve
Claude Code), routes each request by the strategy you pick (single / failover /
roundrobin / timewindow / quota — outside the single strategy, a failed request
is replayed against the next candidate instead of handed back to you), meters
what it cost, and records what happened. Takeover backs up each agent's original
config and restores it when you switch the agent off.

Why I built it: keys were scattered across a GUI, a terminal and a config
file; provider pricing was a table nobody remembered; and a dead provider meant
an afternoon of copy-paste. So the app owns the keys (owner-only local SQLite,
0700/0600), the gateway owns the routing, and the models shelf (24 providers,
synced from a metadata-only Hub) owns the pricing — nothing leaves your machine.

30-second demo: https://youtu.be/M0pO79Wlx-s

Trust worth having, and it is free to check:
every release carries SHA256SUMS and a signed build-provenance attestation,
verifiable against GitHub's transparency log without trusting your mirror:

    gh attestation verify Kiwano_x64.dmg --repo lightconsen/kiwano

No analytics, no telemetry, GPLv3+ (selected modules ported from cc-switch, MIT,
credited in THIRD-PARTY-NOTICES). macOS builds are signed+notarized; Windows
is not yet code-signed, so SmartScreen will warn — known gap, on the list.

Also runs headless on a server (app-less gateway + CLI, user-level service):

    curl -fsSL https://hub.kiwano.cc/install.sh | sh

Feedback welcome — what strategy is missing, which agent config is broken, or
what your cost panel is not telling you.
```

## 附:正文发帖格式提示(发帖前必读)

HN **完全不渲染 markdown**,上面正文里唯一能带过去的结构是:

- **空行分段** —— 照抄即可。
- **行首 4 个空格 = 等宽代码块** —— 上面两条命令(`gh attestation verify` /
  `curl … install.sh`)就是靠这个排版,保持缩进,别改成反引号。
- **反引号是字面字符** —— HN 会把 `single` 原样显示成带反引号的文本,看着像没写完。
  提交前把所有反引号删掉,需要强调就把 single 改成大写 SINGLE。
- **没有列表语法** —— `- ` 开头的行在 HN 上就是普通文字加一个横杠,不会变成圆点。
  正文里没有列表,别加;真要列,改成短段落。
- **链接直接贴裸 URL** —— HN 自动识别。URL 后紧跟的括号注释会被当成同一段文字,
  所以别在链接后面写注释,要说的并进正文。
- `—` 破折号、`×` 都没问题;`---` 分隔线在 HN 不渲染,换成空行。

### 粘贴前必须手改的三处

1. ~~`30-second demo: https://youtu.be/…`~~ —— **已填真实链接**
   `https://youtu.be/M0pO79Wlx-s`(2026-09-20 已上传),占位符已不存在,
   正文里那行可以直接粘贴,不用再改。
2. 视频链接的位置:放第一段之后(现在就在),那是 HN 上最显眼的位置。
3. 通读一遍全文,把任何反引号、`[…]`、中文备注清干净——HN 上没法回头编辑出体面感。

## 发布后清单

- [ ] 前 2 小时逐条回评论,语气谦逊、给具体选择而非辩解
- [ ] 预答对比题:cc-switch / LiteLLM / claude-code-router(措辞见 `01-pitch.md` FAQ)
- [ ] 评论区有人问就跑 `gh attestation verify` 给截图,这招比文字有说服力
- [ ] 同日把 thread 发到 X(见 `04-x-thread.md`),互相引流
- [ ] 次日热点落完后,把评论区高频问题补进 README / `01-pitch.md`