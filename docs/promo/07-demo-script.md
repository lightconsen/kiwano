# 07 — 演示视频分镜(成片 30 秒)

用途:Show HN 首帖视频、X 首条 clip、落地页首屏嵌帧、Reddit 附链。
画面文案用英文(出圈面更大);给中文版时把 `CAP` 行的文字按括号译名替换即可。
全片原则:**不旁白** —— 所有信息上字幕,观众 90 秒内只看得到画面,静音也能懂。

## 片头来一段 3 秒黑场 + logo(0:00–0:03)

`CAP`:KIWANO — LOCAL-FIRST AI PROVIDER MANAGER

## Shot-by-shot(时间连续,无缝剪辑)

| # | 时间 | 画面 | 屏幕上方 CAP(英文) |
| --- | --- | --- | --- |
| 1 | 0:03–0:10 | 打开 Kiwano,停在 Apps 屏:左侧 Provider 列表(用户名结尾模糊处理),右侧各 Agent 卡片带用量 | `A LOCAL AGENT GATEWAY` |
| 2 | 0:10–0:22 | 点开「Add provider」→ 从 Models shelf 搜索(e.g. `DeepSeek`,搜出官方/聚合候选)→ 填一个 mock key `sk-••••` → 保存 | `REGISTER ONCE. 24 PROVIDERS ON THE SHELF.` |
| 3 | 0:22–0:38 | 回到 Apps,把 Claude Code、Codex、Gemini CLI 三张卡的接管开关逐个拨到 ON(注意镜头跟手,卡片出现「Backed up」小標記) | `TAKEOVER IN ONE CLICK. ORIGINALS BACKED UP.` |
| 4 | 0:38–0:52 | 终端里两条命令并排:`claude` 发一个 prompt,另一窗 `codex` 也发一个;切到 App 的 Request logs 看到两条请求同时刷过 | `TWO AGENTS, ONE PORT.` |
| 5 | 0:52–1:05 | 打开某个 Provider 的路由设为 failover;把它的 key 临时改为错钥(或断网),重发请求——App 显示请求自动打到下一个候选,Agent 侧无感知 | `FAILOVER: THE REQUEST REPLAYS, THE AGENT NEVER NOTICES.` |
| 6 | 1:05–1:20 | 成本面板:今日/7 日/30 日切换,按 agent × provider 归因的堆叠柱;quota 环告警;一个「forecast」气泡提前报超限 | `REAL PER-REQUEST COST, PER AGENT × PROVIDER.` |
| 7 | 1:20–1:33 | 终端:`curl -fsSL https://hub.kiwano.cc/install.sh \| sh`,随后 `kiwano agents takeover`,systemd status 显示 active | `HEADLESS MODE: ONE-LINE INSTALL, USER-LEVEL SERVICE, NO ROOT.` |
| 8 | 1:33–1:40 | 黑场收尾:logo + `kiwano.cc` | `GPLv3 · macOS / WINDOWS / LINUX · SIGNED PROVENANCE ON EVERY RELEASE` |

## 制作规格

- 画幅/清晰:录 4K 或 Retina 全分辨率(≥ 2880×1800),导出时缩到 **1920×1080**;
  同时出一个 **1:1 方形 teaser**(30 秒,只含 1+3+4+6 四个镜头)给 X 首条。
- 编码:H.264 mp4,码率 ≥ 8 Mbps;Show HN 和 Reddit 对 mp4 无要求,X 需要 mp4。
- 字幕:底部三分之一,**≥ 52px**,粗体、白字黑描边;每屏停留 ≥ 3.5s 才换。
- 收音:静音即可(全片无旁白);macOS 录音时**关掉所有系统提示音**,避免菜单"叮"。
- 素材真实性:优先用**开发构建 + 内置样例数据**(README 截图同源,保持一致);
  若用真账号,一律 mock key(`sk-••••`),Provider 名可留真名。
- 敏感:任何画面里的 key、token、真实会话正文——不出现。录前通读一遍,
  把可能露 session 内容的窗口先切走。
- 时长:总片 90s±5s;若某镜头难拍,砍 5 和 7 的时长,别砍 4 和 6。

## 发布后

- [x] **已上传 YouTube**(2026-09-20):**https://youtu.be/M0pO79Wlx-s**
      已填进 `02-show-hn.md`、`03-reddit.md`、`04-x-thread.md`、`05-v2ex.md`、
      `06-cc-switch.md`,并把链接写进 `01-pitch.md` 事实面作为唯一来源。
- [ ] 落地页首屏嵌入这个视频(`site/index.html` hero 区),别只留在评论区
- [ ] B 站同步一份(中文渠道用 B 站链接更友好),有了就替换 `05-v2ex.md` 那行
- [ ] 可在 GitHub README 顶部也放一个贴图链接(图片直链需固定,别挂 release 临时文件)

## 成片制作记录(2026-09-20)

**中间源片**(不入库):Playwright 录出的 webm,落在 `/tmp/kiwano-video/kiwano-demo-90s.webm`
(~97s / VP8 / 1600×1000)。**90s 的 mp4 版本已删除**,仓库只留 30s 成片——中间片是重录产物,
留着是负担,需要时一条命令从 webm 直出。

**成片**:`docs/media/kiwano-demo-30s.mp4`(30.3s,**1.5 MB**)+ 封面帧
`docs/media/kiwano-demo-poster.png`。从中间源片**原速剪出**(不做变速,字幕仍可读),七段硬切。

| 成片内时间 | 取自源片 | 内容 |
| --- | --- | --- |
| 0.0–1.3 | 1.7–3.0 | 片头 KIWANO |
| 1.3–4.8 | 7.0–10.5 | Apps 首屏 + **A LOCAL AGENT GATEWAY** |
| 4.8–11.3 | 36.5–43.0 | 策略下拉展开 + FAILOVER 字幕 |
| 11.3–17.8 | 52.5–59.0 | Dashboard 成本 + KEYS NEVER LEAVE 字幕 |
| 17.8–21.8 | 62.5–66.5 | 日志详情弹窗 + AUDITABLE 字幕 |
| 21.8–26.3 | 75.0–79.5 | **Settings → Features 列表** + OPT-IN FEATURES 字幕 |
| 26.3–30.3 | 90.5–94.5 | 片尾 kiwano.cc / GPLv3 |

复录并重剪(从 webm 一步直出 30s 成片,不再生成 90s mp4 中间片):

```bash
# 1) 重录(需要 dev server 在 :1420)
cd app && node node_modules/vite/bin/vite.js --port 1420 --strictPort &
node scripts/record-demo-video.js        # 会打印每条字幕的精确时间戳,如 CAP 40.7s | FAILOVER: …

# 2) 从 webm 按时间戳 trim+concat 直出 30s mp4(改时间戳即可重剪)
ffmpeg -i /tmp/kiwano-video/kiwano-demo-90s.webm \
  -f lavfi -i anullsrc=channel_layout=stereo:sample_rate=44100 \
  -filter_complex "[0:v]split=7[a][b][c][d][e][f][g];\
[a]trim=start=1.7:end=3.0,setpts=PTS-STARTPTS[v0];[b]trim=start=7.0:end=10.5,setpts=PTS-STARTPTS[v1];\
[c]trim=start=36.5:end=43.0,setpts=PTS-STARTPTS[v2];[d]trim=start=52.5:end=59.0,setpts=PTS-STARTPTS[v3];\
[e]trim=start=62.5:end=66.5,setpts=PTS-STARTPTS[v4];[f]trim=start=75.0:end=79.5,setpts=PTS-STARTPTS[v5];\
[g]trim=start=90.5:end=94.5,setpts=PTS-STARTPTS[v6];\
[v0][v1][v2][v3][v4][v5][v6]concat=n=7:v=1:a=0[outv]" \
  -map "[outv]" -map 1:a -c:v libx264 -preset slow -crf 20 -pix_fmt yuv420p -r 25 \
  -c:a aac -b:a 64k -shortest -movflags +faststart docs/media/kiwano-demo-30s.mp4
```

**30s 版是唯一成片**,X / 落地页 / Show HN / Reddit 全部用它。若日后确实需要长版,
从 webm 直接整片转 mp4 即可(不入库,直接传视频站):
`ffmpeg -i in.webm -c:v libx264 -crf 20 -pix_fmt yuv420p -c:a aac -shortest -movflags +faststart out.mp4`

> 为什么不再留长版:30s 覆盖全部卖点且字幕读得完;长版在 HN/Reddit 上的完播率
> 明显更低,而仓库里多一个 4 MB 二进制是长期负担。视频站上传用 30s 版即可。

**制作方式**:全自动录屏,零人工、零后期。

1. 起前端 dev server(`app/` 下 `vite --port 1420`,纯浏览器模式走内置样例数据);
2. Playwright 以 `recordVideo` 开 1600×1000 上下文,并 `addInitScript` 向页面注入三样东西:
   全屏片头/片尾遮罩、底部字幕条(`__kwCap`)、跟随鼠标的圆点光标 —— **字幕是录进去的,不用剪辑**;
3. `scripts/record-demo-video.js` 按本文件的时间轴驱动交互(导航、搜索、展开策略下拉、点开日志弹窗),
   每段 hold 由 `PACE=1.6` 拉长,总长落在 ~92s;
4. 产出 webm(VP8),再转 H.264:`ffmpeg -i in.webm -c:v libx264 -crf 20 -pix_fmt yuv420p
   -movflags +faststart -f lavfi -i anullsrc -c:a aac -shortest out.mp4`。

**复录**(UI 改动后两分钟重出一版):

```bash
cd app && node node_modules/vite/bin/vite.js --port 1420 --strictPort &   # 先起 dev server
node scripts/record-demo-video.js          # 需要 playwright + Chromium(本机已备)
# 转 mp4 用任意带 libx264 的 ffmpeg;本机静态版在
# /tmp/ffbin/darwin_arm64/ffmpeg(临时),或 brew install ffmpeg
```

**与本分镜的差异(有意取舍,不是遗漏)**:

- 首屏字幕用 **A LOCAL AGENT GATEWAY**,不写 `127.0.0.1:8317` —— 端口是可配置的,
  字幕写死具体值,未来默认端口一改就自相矛盾;泛化表述永远不会被打脸;
- 源片在分镜 8 镜之外**新增了 Settings → Features 镜头**(日志镜头之后),展示
  Cost forecast / Anomaly detection / Agent self-query (MCP) / Rule injection 等开关列表;
  字幕写 "OPT-IN FEATURES, BUILT ON THE REQUEST LOG" —— **刻意不说 "all off"**,
  dev 夹具里部分开关是开着的,字幕与画面矛盾比不展示更糟;
- 本片不含 Shot 4(两个真实 agent 发请求)和 Shot 7(install.sh + systemd):本机是 macOS,
  没有 systemd,也没有真实 API key —— **终端画面不造假**,这两个镜头留给真机实拍
  (任何 Linux 服务器上 `asciinema` 或系统录屏,拍完拼在片尾即可);
- 字幕条刻意避开与画面对不上的数字(样例货架 9 条 vs 线上 Hub 24 家),宁可少写不写错;
- 字幕停留、镜头长度由 `PACE` 一个系数控制,想出 60s 短版:`PACE=1.05 node scripts/record-demo-video.js`。