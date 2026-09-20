# 08 — 发布前门面清单

发布任何渠道前,逐项勾完。"产品已就绪"≠"门面已就绪";下面每项不补齐,
流量进来就是折损。

## A. 资产(第 0 号优先级)

- [x] **演示视频** —— **已产出**(2026-09-20):`docs/media/kiwano-demo-30s.mp4`
      (30.3s,H.264 1600×1000,静音轨,+faststart,**1.5 MB**)+ 封面 `kiwano-demo-poster.png`。
      七段原速硬切:片头 → A LOCAL AGENT GATEWAY → 五种策略/FAILOVER → 成本 +
      KEYS NEVER LEAVE → 日志审计 → Settings Features 列表 → 片尾。
      分镜见 `07-demo-script.md`;复录/重剪方法见该文件「成片制作记录」。
      **90s 版本已删除**,30s 是唯一成片,所有渠道统一用它。
      **已上传 YouTube(2026-09-20):https://youtu.be/M0pO79Wlx-s**,
      已填入全部 5 份渠道正文 + `01-pitch.md` 事实面。
      剩余待办:落地页首屏嵌入(见 A 节下一条)、B 站可选补一份。
- [~] **截图** —— 5 张已就位(`docs/screenshots/`,2026-09-20 从当前构建 + 内置样例数据实拍,
      2800×1800 @2x,深色主题,风格统一):
      - `apps.png`(**已重拍**:Apps 全列表,17 providers · 6 agents bound + quota 环。
        旧图页脚写着 "API keys stay in the system keychain",与 README 的 owner-only SQLite
        说法矛盾、且该文案在现代码里已不存在,所以必须换)
      - `routing.png`(策略下拉:single/failover/roundrobin/timewindow/quota)
      - `costs.png`(Dashboard:成本归因 + 7/30 日趋势 + 按 provider/agent 分成)
      - `shelf.png`(Models shelf 全列表 + 分类筛选)
      - `logs.png`(单条请求的完整记录弹窗:脱敏头 + 请求/响应正文 + Copy)
      - `server.png`(终端 `kiwano agents takeover` + `systemctl --user status`)—— **待实拍**
      - `import-cc-switch.png`(终端 `kiwano import cc-switch`)—— **待实拍**
      这两张是终端画面,**必须在装了真实网关的机器上实拍,不要用假终端图**——
      server 模式是这个项目对 selfhosted 社区的门票,一张对不上的截图会毁掉它。
      拍法:任意服务器上 `curl -fsSL https://hub.kiwano.cc/install.sh | sh`,
      然后 `kiwano agents takeover` + `systemctl --user status kiwano`,`asciinema` 或系统截屏皆可。
      已知小疵:样例目录(shelf)是 9 个条目的 fixture,线上 Hub 是 24 个;发帖前若在意,
      把 fixture 补齐或改用真机数据再重拍 `shelf.png`。
      复拍脚本:`/tmp/kshots/capture.js`(需 `pnpm dev` 起在 :1420,纯浏览器即出样例数据)。
- [ ] **落地页首屏放视频**(发布后 24h 内)。
- [x] README 顶部加视频链接 —— **已加**(2026-09-20,`https://youtu.be/M0pO79Wlx-s`,
      放在首段"keys stay on your machine"之后、截图之前;用文字链接而非贴图,
      因为图片直链要挂 CDN 才稳,仓库里的相对路径在 GitHub 之外的渲染器上不可靠)。

## B. 信任门面(repo 层面)

- [x] `SECURITY.md` —— **已写**(2026-09-20)。私有披露走 GitHub Security Advisories
      (不暴露邮箱);scope 里点名了凭证存储、loopback 网关与 /metrics、日志正文、
      更新通道与 Hub 同步;附上 `gh attestation verify` 的验证命令。
- [x] `CONTRIBUTING.md` —— **已写**。含 dev setup、`pnpm build:sidecar` 与"Windows
      保留文件名"两个坑、CI 那四条检查、PR 约定(CHANGELOG `[Unreleased]`、
      i18n 全语言补齐)。
- [x] **issue templates** —— **已写**(`.github/ISSUE_TEMPLATE/`):`bug.yml`
      (版本/平台/区域 + 复现步骤 + 三个自检项,含"先脱敏再粘贴"提示)、
      `feature.yml`(先问"你想做什么"再问功能)、`config.yml`
      (`blank_issues_enabled: false`,把提问与 idea 导向 Discussions)。
      > 有意没做 `question.yml`:提问类进 Discussions 更合适,做模板等于把人
      > 从 Discussions 拉回 issue 列表。
- [x] **README 加「对比段」** —— **已加**(2026-09-20,`## How it compares`):
      Kiwano / cc-switch / LiteLLM 三列对比 + 一句立场(前者是后者子集,
      `import` 一条命令迁移)。文末加了 Contributing / Security 入口。
- [ ] Discussions 开放了(现在是开着的),建一个 `#show-and-tell` 频道,让人晒路由
      配置和账单截图——这是活社区的样子。**需人工在 GitHub 网页建,我不能代做。**

## C. 发布操作

- [ ] Windows 代码签名(**发布前若来不及,所有面向 Windows 的推广标题先避重就轻**
      ——SmartScreen 警告是真实转化杀手)。
- [ ] 预留发布账号的资历:Show HN 用主账号发(别用新注册号);reddit 目标 sub 先攒
      karma(≥ 某门槛,见 `03-reddit.md` 先决条件)。
- [ ] 各渠道链接最终核对:R2 下载直链、landing、仓库 —— 与 `01-pitch.md` 事实面一致。
- [ ] **文案里的数字逐条对事实**(2026-09-20 已核一轮,发帖当天再核一次):
      - Hub 目录 = **24 个 Provider**(23 官方 + OpenRouter)。已实测
        `curl -s https://hub.kiwano.cc/catalog.json` → `total: 24`。
        材料原先写的 19 是过期数字,已全部改正;README 同步修正。
      - 接管 Agent = **14 个**(README 的名单为准)。
      - 端口 `127.0.0.1:8317`;最新版 **v0.1.16**(2026-09-19)。
      - Show HN 标题 **≤80 字符**(原稿三条分别是 90 / 87 / 105,全部超限,已换成实测过
        长度的三条,见 `02-show-hn.md`)。
- [ ] 下载页(DMG/exe/AppImage/deb/rpm + SHA256SUMS + attestation 示例)在**三平台各
      真装一遍**,别发帖后发现桌面版有个启动崩。
- [ ] 运营备注:github.com 操作走 `127.0.0.1:1087` 代理;发布当天把 CI 状态盯一眼
      (README 徽章别在发布日挂红)。

## D. 节奏(具体日历)

以 2026-09-20(周日)为基准。HN 的黄金窗口是**周二~周四 UTC 14:00–17:00 = 北京 22:00–次日 01:00**。

| 日期 | 动作 |
| --- | --- |
| 9/20(日) | 勾完 A / B 两节(视频、截图、SECURITY / CONTRIBUTING / issue 模板、README 对比段) |
| 9/21(一) | 提交 awesome-tauri(无门槛);awesome-claude-code 的 14 天门槛今天起满足,可提交 |
| 9/22(二) | cc-switch Discussions 迁移帖(暖场,精准受众);同日围观回帖 |
| 9/24(四) 22:00 | **Show HN** + X thread 同日(视频就绪为前提) |
| 9/26(六) 20:00–22:00 | V2EX《我用 Rust 写了一个本地优先的 AI 网关》 |
| 9/27–9/28 | Reddit(与 HN 错开 2~3 天);awesome-selfhosted 提 PR,借 HN 余热 |
| 其后 4~6 周 | 0.1.17 起每版挑一个亮点单独发帖;周日晚汇总下载 / star / 外部 issue |

- [ ] 每个下一个小版本(0.1.17…)挑一个亮点单独发一帖,保持 4~6 周曝光。
- [ ] 周日晚汇总:R2 下载数、star、外部 issue,更新本清单的"已发布"状态。