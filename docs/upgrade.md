# Kiwano 应用在线更新规划（docs/upgrade.md）

> 目标：为 Kiwano 桌面应用接入官方 `tauri-plugin-updater` 自动更新能力，
> 覆盖签名密钥、应用侧接入、前端 UI、发布流水线与测试验证。
> 现状基线：v0.1.0，无任何 self-update 机制（无 updater 插件、无 CI、无签名配置）。

## 0. 现状与约束

| 项 | 现状 | 对本规划的影响 |
|---|---|---|
| 更新机制 | 无（`Cargo.toml` / `package.json` 均无 updater 插件） | 全量新增 |
| CI/发布 | 无 `.github/workflows` | 需从零建发布流水线 |
| 打包 | `bundle.targets: "all"`，macOS 为主（`macOSPrivateApi`、Overlay 标题栏） | updater 三平台支持面不同，见 §3 |
| 前端 API | `src/api/tauri.ts` 与 `lib.rs` 命令 1:1 映射，另有 `dev.ts` 浏览器 mock | 新增命令需同步 `types.ts` / `tauri.ts` / `dev.ts` |
| 守护进程 | gateway daemon 存活期长于 GUI，重启后经 admin ping 收养（`lib.rs` 底部注释） | 更新重启天然安全，无需额外处理，验收时确认即可 |
| 隐私立场 | local-first；Hub 同步协议已实现但禁用（`lib.rs:568`），API 请求与密钥不过 Hub（spec §6.1） | 更新检查应默认可关、失败静默、无遥测 |

## 1. 方案选型

**采用 Tauri 2 官方 updater 插件**（`tauri-plugin-updater`），manifest 托管在 **GitHub Releases**：

- 插件内置 minisign 签名验证，是 Tauri 2 的标准路径；
- `tauri-action` 在 CI 中自动产出各平台 updater 工件与 `latest.json`，无需自建服务器；
- 备选：静态托管到 `hub.kiwano.cc`（复用 Hub 域名，协议 v0 本就是静态 JSON，风格一致）。
  端点只是配置项（`tauri.conf.json` 的 `endpoints` 数组），先走 GitHub Releases，后续可平滑切换或叠加。

更新检查形态：**设置页手动「检查更新」为常态入口；启动时静默检查为可选项（默认开启、失败完全静默）**。
插件仅拉取静态 JSON，不带遥测，符合 local-first 立场。

## 2. 阶段 0 — 签名密钥（前置，人工操作）

1. 本地生成 minisign 密钥对：

   ```bash
   pnpm tauri signer generate -w ~/.tauri/kiwano.key
   ```

2. 保管约定：
   - **私钥与密码只存本地 + GitHub Actions Secrets**（`TAURI_SIGNING_PRIVATE_KEY`、`TAURI_SIGNING_PRIVATE_KEY_PASSWORD`），绝不入库；
   - 公钥写入 `tauri.conf.json`（`bundle.updater.pubkey`），随仓库提交；
   - 私钥丢失 = 已发布用户无法再收到验证通过的更新，密钥需离线备份。

## 3. 阶段 1 — 应用侧接入（Rust / 配置）

### 3.1 依赖

```toml
# src-tauri/Cargo.toml [dependencies]
tauri-plugin-updater = "2"
tauri-plugin-process = "2"   # relaunch 用
```

```jsonc
// package.json dependencies
"@tauri-apps/plugin-updater": "~2",
"@tauri-apps/plugin-process": "~2"
```

### 3.2 权限（`src-tauri/capabilities/default.json`）

```jsonc
"permissions": [
  // ...现有项不动
  "updater:default",
  "process:allow-restart"
]
```

### 3.3 插件注册（`src-tauri/src/lib.rs` builder 链）

```rust
// Self-update (docs/upgrade.md): static manifest on GitHub Releases;
// check is opt-out at startup, always manual from Settings.
.plugin(tauri_plugin_updater::Builder::new().build())
.plugin(tauri_plugin_process::init())
```

### 3.4 打包配置（`src-tauri/tauri.conf.json`）

```jsonc
"bundle": {
  "createUpdaterArtifacts": true   // 产出 *.tar.gz / *.exe + *.sig 及 latest.json 所需签名
}
```

> 注意：三平台 updater 支持面 —— macOS `.app.tar.gz`、Windows NSIS `.exe`、Linux 仅 AppImage
> （deb/rpm 由系统包管理器负责，不做热更新）。现有 `targets: "all"` 不用改。

## 4. 阶段 2 — 命令与前端 UI

### 4.1 Rust 命令（沿用 1:1 命令映射惯例，网络 IO 走 async 命令线程）

新增两个命令并注册进 `generate_handler!`：

- `check_app_update() -> Option<UpdateInfoVm>`：`app.updater_builder()?.check()?`；返回 `None` 即已是最新。
  `UpdateInfoVm { version, notes, pub_date }`（进 `vm.rs`，与现有 `*Vm` 命名一致）。
- `download_and_install_app_update() -> bool`：下载 + 校验 + 落盘 + `relaunch()`。
  下载进度通过 `app.emit("update://progress", ProgressVm { downloaded, total })` 推送。

集中放在新模块 `src-tauri/src/update.rs`，`mod` 挂进 `lib.rs`。

### 4.2 前端（三件套同步改）

- `src/api/types.ts`：新增 `UpdateInfo`、`UpdateProgress` 类型；
- `src/api/tauri.ts`：`checkAppUpdate` / `downloadAndInstallUpdate` + `listen("update://progress")`；
- `src/api/dev.ts`：mock 返回 `null`（浏览器 dev 永远"已是最新"）。

### 4.3 UI（`src/screens/Settings.tsx`）

设置页新增「关于 / 更新」区块（现有 `Row` 结构即可）：

- 当前版本号（`getVersion()` from `@tauri-apps/api/app`）；
- 「检查更新」按钮 → 三态：已是最新（inline 提示）/ 有新版（版本号 + 更新日志 + 「下载并安装」）/ 失败（错误行内展示）；
- 下载中显示进度条（进度事件驱动），完成后自动 relaunch；
- 「启动时检查更新」开关，落到 `ui_settings`（`vm.rs` 增字段，默认 `true`）；
- UI 字符串保持中文（项目约定）。

启动静默检查放在 `lib.rs` setup 末尾：spawn 异步任务，失败静默，命中新版仅发托盘/系统通知
（复用 `tauri-plugin-notification`），不打断首屏。

## 5. 阶段 3 — 发布流水线（`.github/workflows/release.yml`）

- **触发**：push tag `v*`（如 `v0.2.0`），`workflow_dispatch` 手动补发；
- **矩阵**：`macos-latest`（aarch64 + x86_64）、`ubuntu-22.04`、`windows-latest`；
- **核心步骤**：checkout → pnpm + Rust 工具链 → 前端依赖/构建由 `tauri-action` 托管 →
  `tauri-apps/tauri-action@v0`（`tagName`、`releaseDraft: true`、`includeUpdaterJson: true`）；
- **Secrets**：`TAURI_SIGNING_PRIVATE_KEY` / `..._PASSWORD`、`GITHUB_TOKEN`；
  macOS 追加 Apple 签名公证 secrets（`APPLE_CERTIFICATE` 等）——**updater 只验 minisign 签名，
  不解决 Gatekeeper**，macOS 分发要过公证需另行配置（可作为后续迭代，v0 先 developer 自用）；
- 产出 draft release：各平台安装包 + `.sig` + `latest.json`；所有平台上传完毕后由 `publish`
  job 自动转正（推 tag 是唯一的人工动作），避免半成品 release 与残缺 `latest.json` 外泄；
- 镜像：安装包与 `latest.json` 同步到 R2（`hub.kiwano.cc/releases/`，文件名不带版本号）。

配套：版本号目前散在 `package.json` / `tauri.conf.json` / `Cargo.toml` 三处（均为 0.1.0），
加一个 `scripts/release.sh`（或 Makefile 目标）一次性 bump 三处 + 生成 tag，避免漂移。

## 6. 阶段 4 — 测试与验证

1. **本地全链路**：bump 版本 → `pnpm tauri build` 得到签名工件 → 本地起静态服务器托管 `latest.json` +
   工件 → 旧版应用把 endpoints 临时指向 `http://127.0.0.1:PORT/latest.json` → 走完检查/下载/重启；
   （updater 强制 HTTPS 仅对生产端点，本地 http 可用于调试，发布前移除）
2. **签名负例**：篡改工件或换错 pubkey，确认下载被拒；
3. **收养回归**：更新重启后确认 gateway daemon 被 admin ping 收养、托盘与 autostart 正常；
4. **失败静默**：断网启动，确认无弹窗、无报错噪声；
5. **平台矩阵**：macOS（主）/ Windows NSIS / Linux AppImage 至少各过一遍安装→更新。

## 7. 验收清单

- [ ] 密钥对生成并离线备份，公钥入库，Secrets 配置完成
- [ ] `Cargo.toml` / `package.json` / capabilities / `lib.rs` / `tauri.conf.json` 五处接入完成
- [ ] `update.rs` 两命令 + `UpdateInfoVm`，`types.ts` / `tauri.ts` / `dev.ts` 同步
- [ ] 设置页「关于/更新」区块：版本号、手动检查、进度条、relaunch、启动检查开关
- [ ] `release.yml` tag 触发构建，含 `latest.json` 与 `.sig`，全平台就绪后自动 publish
- [ ] 版本 bump 脚本统一三处版本号
- [ ] §6 五项测试全部通过

## 8. 开放问题

- macOS 公证（Developer ID + notarization）是否进 v0？建议 v0 先跳过（自用/内测），公开发布前补；
- `latest.json` 端点是否同步镜像到 `hub.kiwano.cc`？建议等 Hub 上线时一并做，端点数组支持多源容灾；
- 是否引入「跳过此版本」？建议 v0 不做，保持最小实现。
