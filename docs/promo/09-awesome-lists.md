# 09 — 列表站 / awesome 收录

用途:把 Kiwano 提交进开发者真正会浏览的收录列表。这是长尾里**性价比最高的一档**:
一次提交、零维护,之后长期被动带量;而且这些列表的收录本身会变成"可信度背书"。

与 `02`~`06` 的渠道不同,这里的规则由各列表自己定,而且**违规会被直接关单**。
所以下面每条都写了已核实的具体门槛和提交路径。规则可能变,**投稿前再读一次该列表的
CONTRIBUTING**。

---

## 已核实的三个列表(2026-09-21 复核)

> 复核结论:**awesome-tauri 出局**(不再收应用),实际只剩两个可投。
> 规则会变,投稿前再读一次该列表的 `.github/contributing.md` 与 PR 模板。

| 列表 | 提交方式 | 硬门槛 | Kiwano 现状 | 最早可提交 |
| --- | --- | --- | --- | --- |
| awesome-claude-code | **网页 issue 表单**,不是 PR | ≥14 天(首个 commit 起)且有持续开发 **或** ≥100 star | 首 commit 2026-09-07,持续开发中,0 star → 走 14 天路线 | **2026-09-21** |
| awesome-tauri | ~~PR~~ | **不再接受应用投稿**(2026-08-24 起) | Kiwano 是桌面应用 → **不投** | — |
| awesome-selfhosted | PR 到 **`-data` 仓库** | FOSS + 可自托管 + 有许可证 + 持续维护 | GPL-3.0、headless 网关可自托管 | 随时(建议 HN 之后) |

---

## A. awesome-claude-code

仓库:`hesreallyhim/awesome-claude-code`

### 规则(原文要点,别踩)

- 推荐的门槛是**二选一**:①首个 commit 起 **满 14 天** 且看得到持续开发;②**≥100 star**。
  不满足的直接被机器人自动关单。
- **一次只能推荐一个资源。**
- **必须走网页 issue 表单**(`issues/new?template=recommend-resource.yml`),
  **不要开 PR**;而且**不能用 `gh` CLI 提交**——只能人工在浏览器里填。
- 描述要求:写成**陈述句**,不要推销腔、不要对着读者说话、**不要 emoji**、**一行**。
- 闭源可投但难审;**需要注册或付费的一律不接受**(Kiwano 开源免费,没问题)。
- 维护者明说:别把"上榜"当推广主策略,它只保证列入候选。**因此这一步是补充,不是主战场。**

### 时间点

Kiwano 首个 commit 是 **2026-09-07**,14 天门槛在 **2026-09-21** 满足。
→ **9/21 起就能投**,不用等 star。

> ⚠️ 实测(9/21):机器人按 **14×24 小时**精确计时(基准是仓库 `created_at`
> 2026-09-07T05:59:31Z,整点 = 北京 13:59:31),**不按日历日**。11:08 提交的
> #2898 因差 2 小时 51 分被自动关闭;14:23 重提的 **#2901** 一次通过。
> 任何「N 天门槛」的列表都按这个口径估时间,别按日历日提前交。

### 建议填写的描述(一行、陈述句、无 emoji)

```
Local-first AI provider manager: a desktop app plus a local gateway that routes Claude Code, Codex and 12 more agents through the providers you already have, with failover and per-request cost metering.
```

> 提交时如果是走"14 天 + 持续开发"这条,可以在表单里补一句事实:
> "First commit 2026-09-07; active development since (see CHANGELOG.md, releases through v0.1.16)."
> 不提 star,诚实走第一条路线。

### 被收录后

README 顶部可挂官方徽章(维护者邀请,不是必须):

```markdown
[![Mentioned in Awesome Claude Code](https://awesome.re/mentioned-badge.svg)](https://github.com/hesreallyhim/awesome-claude-code)
```

---

## B. awesome-tauri —— **不投**(规则已变,2026-09-21 复核修正)

仓库:`tauri-apps/awesome-tauri`(默认分支是 `dev`,不是 `main`)

> **2026-08-24 起该列表不再接受应用投稿。**
> `.github/contributing.md` 第一行:**"Application submissions are not accepted
> anymore!"**;PR 模板第二条是同一句话的勾选项;仓库里还有一条 `remove-apps` 分支在
> 清理存量应用条目。
>
> 本文 9/20 那版写的"分类:Applications → Developer tools / Apps 类无年龄门槛"是
> **错的**——当时 README 里已经没有 Applications 分类了,规则在一个月前就改了。
> 照那版去提 PR 会被立刻关闭。**教训:awesome 列表的规则看 `.github/contributing.md`
> 和 PR 模板,不看 README,也不看二手笔记。**

### 剩下的类别与 Kiwano 的关系

| 类别 | 门槛 | Kiwano |
| --- | --- | --- |
| Guides & Tutorials / Articles | 不适合:是文章,不是项目 | — |
| Templates | Tauri 2.x + 开源 + **≥30 天** + 英文文档 | 不是模板 |
| Plugins | 同上 + 是 Tauri 插件 | 不是插件 |
| Integrations | 同上 + 是 Tauri 集成库 | 不是集成库 |

即使想改投这些类别,30 天门槛也要到 **2026-10-07**(首 commit 9/07)才满足,而且
Kiwano 本身不属于任何一类。**结论:放弃这个列表,别硬投。**

### 将来什么情况下能投

只有当你把 Kiwano 里的某块**抽成独立的 Tauri 插件**(比如托盘 / 单实例 / 自动更新那套)
并单独开源时,才以插件身份投 Plugins,且需满 30 天。届时格式:

```markdown
- [Name](https://github.com/…) ![v2] - Description.
```

规则(仍适用):描述 **≤24 词**、无链接无括号、不以 A/An 开头、字母序插入、
一个 PR 一条、行尾不留空格、`backticks` 包名。**commit 签名**:contributing 写
"You have to",PR 模板标 "(optional)" —— 两处矛盾,按严的做,签。

---

## C. awesome-selfhosted

仓库:**提交目标是 `awesome-selfhosted/awesome-selfhosted-data`**

### 规则(原文要点)

- 展示仓库 `awesome-selfhosted/awesome-selfhosted` 是**只读生成物**:
  **PR 和 issue 都被模板拦下**,README 是自动生成的,改它没用。
  所有内容变更都提到 **`awesome-selfhosted-data`**。
- 条目格式(注意许可证和语言用反引号包住):

```markdown
- [Name](homepage) - Description. ([Demo](url), [Source Code](url)) `LICENSE` `Language`
```

- 描述 **≤250 字符**、以句号结尾、不要营销腔。
- 必须是**自由开源软件且有明确许可证**;必须**可自托管**、不依赖第三方服务兜底;
  长期无开发(6~12 个月)会被移出。
- 维护方跑 CI 查死链和许可证,所以 LICENSE 必须规范可识别(Kiwano 是 GPL-3.0 ✓)。

### Kiwano 的切入点

**用服务器形态投,不要用桌面 App 投。** 自托管社区的判断标准是"能不能自己跑起来",
而 Kiwano 的 headless 形态正好是:

- 一条命令安装:`curl -fsSL https://hub.kiwano.cc/install.sh | sh`
- 用户级 systemd 服务,免 root
- `kiwano` CLI 管 Provider / 路由 / 接管 / 日志 / 导出配置
- `/health` 与 `/metrics`(Prometheus 文本)可直接接监控

第一段就把"headless 网关 + CLI"放前面,桌面 App 只作为"同一个 crate 的另一个前端"提一句——
这是 r/selfhosted 和这个列表共同的偏好,顺序写反了会被当成"桌面软件来蹭榜单"。

### 建议条目(插入时按字母序)

```markdown
- [Kiwano](https://kiwano.cc/) - Local-first AI gateway that routes coding agents through the providers you already have, with failover strategies and per-request cost metering. ([Source Code](https://github.com/lightconsen/kiwano)) `GPL-3.0` `Rust`
```

### 投稿前必做

1. 打开 `awesome-selfhosted-data` 的 CONTRIBUTING,确认当前的字段模板与分类名
   (分类结构时有调整,别照抄本文的猜测)。
2. 确认要挂的分类:优先找"Generative AI / LLM 网关"或"API 管理"这类既有分类,
   **不要自己新建分类**。
3. 确认描述里的链接(kiwano.cc、install.sh)都活着。

---

## D. 可选:Product Hunt / AlternativeTo

- **Product Hunt**:dev-tool 在 PH 上通常量级一般,但能拿到首批外部反馈和一次永久页面。
  如果要发,和 Show HN 错开至少一周,别同时抽干注意力。
- **AlternativeTo**:可以把自己挂成 cc-switch / LiteLLM / claude-code-router 的替代选项,
  这是被动长尾流量,填一次即可。**姿态仍是"覆盖更全",不要贬低对方**
  (口径见 `01-pitch.md` 对比表)。

## E. 怎么自己找更多列表(比背清单更有用)

GitHub 上搜 `topic:awesome-list` 加上你的关键词,例如:

- `awesome list claude code`
- `awesome list llm gateway`
- `awesome rust cli`

看到列表后**先读 CONTRIBUTING 再动**,尤其确认三件事:年龄/star 门槛、一次能提几条、
是 PR 还是 issue 表单。**这三样几乎决定了提交是过还是被机器人关掉。**

---

## 执行顺序建议

| 时间 | 动作 |
| --- | --- |
| 即刻 | awesome-tauri 提 PR(无门槛,先落袋) |
| 2026-09-21 起 | awesome-claude-code 走网页表单提交(14 天门槛已满足) |
| Show HN 之后 2~3 天 | awesome-selfhosted 提 PR(HN 有热度时更容易被人工过审) |
| 任意晚些时候 | Product Hunt / AlternativeTo(可选) |

## 附:提交后别做的事

- 别在同一个列表重复提交(会被判刷榜)。
- 别在 PR / issue 里带返利链接或推广位(见 `01-pitch.md`,Kiwano 当前没有任何返利链接)。
- 被拒就接受,按维护者给的理由改;**不要在讨论区争论**——那是公开场合的长期印象。
