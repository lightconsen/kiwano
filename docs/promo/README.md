# Kiwano 冷启动推广包

> **要开始执行,直接打开 `10-launch-runbook.md`** —— 那是按天排好的完整步骤
> (今天做什么、9/21 投哪个列表、9/24 几点发 HN、发完前两小时怎么做)。
> 本文件是总览,`01`~`09` 是各渠道的成稿材料。

状态:2026-09-20,处发布态(v0.1.16),**渠道材料与社区门面均已齐备**。
材料按"发布顺序"编号;`07` 和 `08` 是发布前必须完成的资产与门面。

> 全部材料已于 2026-09-20 对照线上事实核过一轮(见文末「已核验事实」),
> 修正了 Provider 数量、Show HN 标题超限、Show HN 正文的 "8 more" 等错误。
> 发帖前请再核一次数字。

## 打法一页纸

| 渠道 | 素材 | 文件 | 时机 |
| --- | --- | --- | --- |
| Show HN | 帖子全文 + video | `02-show-hn.md` | 视频+截图就绪后,选周二~周四 UTC 14:00~17:00 |
| Reddit | 3 个 sub 的帖子模板 | `03-reddit.md` | 与 Show HN 错开 2~3 天 |
| X / Twitter | thread + clip | `04-x-thread.md` | 与 Show HN 同日或次日 |
| V2EX | 中文帖 | `05-v2ex.md` | 中文圈,第二个周末 |
| cc-switch 社区 | 迁移帖/outreach | `06-cc-switch.md` | 最早做——受众最精准 |
| awesome / 列表站 | 3 个列表的收录条目 | `09-awesome-lists.md` | 长尾被动流量;awesome-tauri 即刻可投 |
| — | 演示视频分镜 | `07-demo-script.md` | 发布前第 0 号资产(已产出) |
| — | 门面清单 | `08-checklist.md` | 发布前全部勾完,含具体日历 |
| **全流程** | **按天执行的步骤** | **`10-launch-runbook.md`** | **从今天到 +6 周,照做即可** |

建议顺序:**cc-switch 迁移帖 → Show HN(视频) → X → Reddit → V2EX**,
awesome / 列表站穿插在长尾里做。**完整日期表见 `10-launch-runbook.md`**。

理由:cc-switch 是可寻址的存量用户,先暖场;Show HN 是主爆点;其余做长尾。

## 七个渠道各自一句话

1. **cc-switch Discussions** —— 直接面向"已经在管多 Provider"的精准用户,且你开源引用过它的代码,话题天然成立。
2. **Show HN** —— dev-tool 在 HN 的主场,30 秒视频是转化主力,帖子要短。
3. **X/Twitter** —— Claude Code 圈流量最大,短视频 clip 传播率最高。
4. **Reddit** —— r/ClaudeAI 是体量最大的相关 sub,但要用"我开源了一个工具"的真诚框架,别广告味。
5. **V2EX** —— 中文开发者对"本地+隐私+开源"三件套接受度高,发到「推广」节点。
6. **GitHub 自身** —— README 对比表、issue templates、SECURITY/Contributing 决定路人"是不是玩具"的第一判断。
7. **awesome / 列表站** —— 一次提交长期被动带量;门槛是硬规则,细则见 `09-awesome-lists.md`。

## 测速指标(前 6 周)

- 周指标:GitHub star、外部 issue 数、R2 下载量(GH release 计数可作代理)。
- 每版发一个小亮点帖(如 0.1.16 的三个新告警),保持曝光节奏。
- 别追 star 本身;盯"下载 → 安装 → 回访/issue"。

## 事实的唯一来源

所有渠道文案的事实(agents 列表、策略、端口、安装命令、providers 数量、许可证)都出自
仓库自身:`README.md`、`docs/cli.md`、`CHANGELOG.md`、`THIRD-PARTY-NOTICES.md`。
**改动文案前先对这四个文件**,不要在渠道文案里发明新说法。

**唯一例外是 Provider 数量** —— 它由另一个仓(模型目录)生成、由 Hub 在线分发,
仓库文档里的数字会滞后。**以线上 `https://hub.kiwano.cc/catalog.json` 的 `total` 为准**
(2026-09-20 为 24)。发帖当天再拉一次,别背数字。

## 语言约定(本目录内)

- `README.md`、`08-checklist.md`、`09-awesome-lists.md` 等"给你自己看的"文件:中文。
- 直接粘贴到英文渠道的正文(`02`~`04`、`06`):英文。
- `05-v2ex.md`:中文。`07` 分镜:画面文案英文、注释中文。
- `09` 里的收录条目:英文原样提交(那是列表的格式要求),其余说明中文。

## 当前缺口(发布前必补,详见 `08-checklist.md`)

> **2026-09-20 收盘状态:渠道材料已 100% 齐(10 份 + 视频 + 5 张截图),
> 社区门面(SECURITY / CONTRIBUTING / issue 模板 / README 对比段)已补齐。**
> 下面是**仍未做**的,共 4 项,其中 2 项只有你能做:

- **落地页首屏嵌入视频**:链接 `https://youtu.be/M0pO79Wlx-s`(已上传,2026-09-20)。
  各渠道正文已全部填好,唯一没做的是 `site/index.html` 的 hero 区。
- **B 站可选补一份**:中文渠道用 B 站链接对国内读者更友好,有了就替换 `05-v2ex.md` 那行。
- **2 张终端截图**(`server.png`、`import-cc-switch.png`):需在真实服务器上跑
  `install.sh` + `kiwano agents takeover` 实拍,**别造假图**——server 模式是
  这个项目对 selfhosted 社区的门票。
- **Windows 代码签名**(未签,SmartScreen 会挡 Windows 用户):来不及就在面向
  Windows 的推广里避重就轻。
- **Discussions 建 `#show-and-tell` 频道**(GitHub 网页人工操作)。

可选优化:样例货架是 9 条 fixture、线上 Hub 是 24 家,`shelf.png` 与文案数字
对不上;介意就把 fixture 补齐再重拍(复拍脚本见 `08-checklist.md` A 节)。

## 已核验事实(2026-09-20 实查,改动文案前先看这里)

| 事实 | 值 | 怎么核的 |
| --- | --- | --- |
| Hub 目录 Provider 数 | **24**(23 官方 + 1 聚合 OpenRouter) | `curl -s https://hub.kiwano.cc/catalog.json` → `total: 24`;`manifest.json` → `catalog.count: 24` |
| 接管 Agent 数 | **14**(名单以 README 为准) | README + `crates/core/src/takeover.rs` |
| 本地网关端口 | `127.0.0.1:8317` | README |
| 最新版本 / 日期 | v0.1.16 / 2026-09-19 | GitHub Releases API |
| 仓库 | 2026-09-07 创建,GPL-3.0,0 star | GitHub API |
| 落地页 | `https://kiwano.cc` 在线,英文优先 + 中文版 | HTTP 200 + 页尾语言字典 |
| 安装脚本 | `https://hub.kiwano.cc/install.sh` 在线(15 KB) | HTTP 200 |
| macOS 签名 | 已 Developer ID 签名 + 公证 | README |
| Windows 签名 | **未签**,SmartScreen 会警告 | README |

> 材料里原先多处写"19 个 Provider",与线上 24 不符,已全部改为 24;
> 仓库根的 `README.md` 同一处过期数字也已同步修正。