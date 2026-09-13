# Kiwano 代码签名与发布可信度（docs/sign.md）

> 目标：记录各平台的签名现状、已经落地的机制，以及**刻意推迟**的决定与触发条件。
> 现状基线：v0.1.7，发布流水线见 `.github/workflows/release.yml`（设计背景见 `docs/upgrade.md` §5）。
> 本文不重复 `upgrade.md` 的更新机制设计，只讲「谁签的、凭什么、还差什么」。

## 0. 现状一览

| 平台 | 产物 | 签名 | 签后校验 | 更新链 |
|---|---|---|---|---|
| macOS | `.dmg` / `.zip` | ✅ Developer ID + 公证 + staple | ✅ 见 §1 | Tauri updater（minisign） |
| Windows | `-setup.exe` / `.msi` | ❌ **未签名** | — | Tauri updater（minisign） |
| Linux | `.AppImage` | ✅ 仅 updater 的 minisign `.sig` | ✅ 客户端验签 | Tauri updater（minisign） |
| Linux | `.deb` / `.rpm` | ❌ 未签名（也无 GPG 源签名） | — | 不参与自动更新 |

macOS 的签名主体是 **个人账号**（`Developer ID Application: Jianxin Yang (7WT4583592)`），
不是组织。这一点在 §3 的 Windows 决策里是硬约束。

三个平台的更新链都靠 **Tauri updater 的 minisign 签名**，与操作系统级的代码签名是两套东西：
`.sig` 文件只保护 App 内更新，对 SmartScreen / Gatekeeper 没有任何作用（见 §1.1）。

## 1. 已经落地的机制

以下六项在本次改动中进入 `release.yml`，均在发布路径上，失败即中断发布。

### 1.1 macOS：签名身份钉死，而非「有签名就算过」

原来只校验 `Authority=Developer ID Application` 这一前缀 —— 而**全世界任何一张
Developer ID 证书都满足它**，所以它只能发现「完全没签」，发现不了「签错了人」。
现在改为三重校验：

- `codesign --verify -R="anchor apple generic and certificate leaf[subject.OU] = \"<TEAM_ID>\""`
  —— 基于 requirement 的校验，把叶子证书钉到本团队；
- 对 `codesign -dv --verbose=4` 输出做**整行精确匹配**
  `Authority=<完整身份>` 与 `TeamIdentifier=<TEAM_ID>`；
- app、DMG、以及 app 内的 gateway sidecar 三者都验，且要求同一身份
  （sidecar 才是真正持有凭据的二进制）。

身份与 Team ID 只在「Prepare the Apple signing material」一处解析并写入 `GITHUB_ENV`，
避免校验值与被签值来自两个可能漂移的来源。

### 1.2 macOS：公证失败不再产生重复提交

`notarytool submit --wait` **不能作为整体重试** —— 第二次调用在 Apple 侧是第二次提交。
一次因网络中断的 `wait` 会让同一镜像排队两个提交，重试就在和自己的前一次赛跑。

现在改为：`submit --no-wait` 取回 submission id → 针对该 id 重试 `wait`。
只有「提交后根本没拿到 id」（传输失败）才允许重新提交。
`Invalid` 视为终局，立刻失败并拉取 `notarytool log`（唯一能给出原因的地方）。

### 1.3 全平台：构建来源证明（build provenance）

新增 `release-manifest` job，对 release 上的每个产物生成 keyless attestation：

```sh
gh attestation verify Kiwano_x64.dmg --repo lightconsen/kiwano
```

无密钥、无采购、走 GitHub OIDC，签名进入 GitHub 的透明日志。它回答的是
「这个二进制是否由本仓库该 commit 的发布流水线构建」—— **不回答「代码是否安全」**。
这一层是校验和、镜像站、release 页面都提供不了的，因为那三者都可以被替换。

产物包含 `latest.json`，所以清单本身也在证明范围内。

### 1.4 全平台：`SHA256SUMS`

在同一个 job 中、对 release 实际携带的资产生成，用 `sha256sum` 原生格式，
`sha256sum -c` 可直接校验。**必须在 staple 之后取**：macOS 的 DMG 在 staple 后被替换过一次，
构建时算出的摘要描述的是没人下载到的字节。

`SHA256SUMS` 只发到 GitHub Release，不进 R2 —— 镜像的路径不带版本号，
放在那里会与文件名对不上。

### 1.5 更新信任根钉到 tag 上

`verify` job 在花掉几分钟构建之前，就把 `src-tauri/tauri.conf.json` 里的
`plugins.updater.pubkey` 与仓库变量 `EXPECTED_UPDATER_PUBKEY` 比对。

updater 的信任根就是这个编译进 App 的公钥：能改它的人可以给所有已安装用户推任意更新，
而改动只是文件里的一行 base64。公钥在 tag 自己的树里，所以自我比对没有意义 ——
比对对象是**仓库变量**，它在仓库之外，只有仓库管理员能改。

> ⚠️ **运维要求**：必须在仓库 Settings → Secrets and variables → Actions → Variables
> 中设置 `EXPECTED_UPDATER_PUBKEY`。未设置时该步骤**故意失败**（失败信息会打印当前值，
> 复制即可）。一个「未配置就跳过自己」的检查，正是攻击者最希望的状态，且它看起来像构建通过。
> 有意不检查时，请删除该步骤，不要改成条件执行。

### 1.6 发布清单的完成早于发布

`publish` 的依赖从 `needs: build` 改为 `needs: [build, release-manifest]`：
`SHA256SUMS` 属于发布内容，发布后再补会让下载窗口内的用户看到一份命名了不完整资产的清单。
镜像 job 依旧不参与依赖 —— 镜像故障不应把 release 卡成 draft。

## 2. 为什么 Windows 现在不买证书

### 2.1 先把两个目标分开

- **目标 A：有发布者身份** —— 安装时不再显示「未知发布者」，AV/EDR 与企业策略
  （AppLocker / WDAC）按发布者放行，用户可验证发布者。**签名能达到。**
- **目标 B：SmartScreen 弹窗消失** —— **签名达不到。**

微软在 2024 年取消了 EV 证书的即时信誉特权，**OV 与 EV 现在在 SmartScreen 上行为一致**；
信誉按**文件哈希**累积，需要数周到数月的干净下载量。
而 Kiwano 一天可能发多个版本，**每个版本的安装包哈希都是新的、信誉归零** ——
高频发布本身就在阻止信誉累积。真正无弹窗的路径只有 Microsoft Store（由微软重签）。

### 2.2 可行性与结论

| 方案 | 可行性 | 成本 | 得到什么 |
|---|---|---|---|
| Azure Trusted Signing | ❌ 个人身份验证已暂停，且个人仅限美国/加拿大 | — | — |
| **SignPath Foundation** | ✅ GPL-3.0 符合其 OSI 许可要求，但信誉评估为自由裁量 | **免费** | Authenticode，**证书主体为 SignPath Foundation** 而非 Kiwano |
| OV 证书 + 云 HSM（SSL.com eSigner / DigiCert KeyLocker） | ✅ 个人可申请 | ~$200–500/年 | 目标 A，**不含目标 B** |
| 不签，文档写清 | ✅ 现状 | 0 | — |

**结论：不急。** 花 $300/年买到的是发布者名字好看，弹窗仍在；对 0.1.x 的项目，
这笔钱的边际收益不成立。README 与 release body 已经写清 SmartScreen 的绕过步骤，
那才是当前最有效的缓解。

## 3. 待办与触发条件

| # | 事项 | 触发条件 | 成本 |
|---|---|---|---|
| 7 | 申请 SignPath Foundation | **现在就可以提交**（审核需时且需信誉积累，被拒无损） | 免费 |
| 8 | 增加 Windows arm64 构建腿 | Windows 用户中出现 arm64 设备反馈 | 免费，~30min |
| 9 | OV 证书 + 云 HSM 签名 | Windows 用户量起来，且**明确抱怨「未知发布者」**（而非弹窗本身） | ~$200–500/年 |

第 7 项需要的前置材料（SignPath 的硬性要求）：公开的自动化构建、
**一份代码签名政策页面**（含 "Free code signing provided by SignPath.io, certificate by
SignPath Foundation." 归属声明）、全员 MFA、以及明确 signing roles（作者/评审/批准人）。

第 9 项落地时的 CI 形态：不要把签名塞进现有 build job，照
[OpenClaw 的做法](https://github.com/openclaw/openclaw-windows-packaging)拆成
build（产出 unsigned）→ sign（挂 `environment:` 加审批人）→ verify（重新读签名比对主体）三段。
Tauri v2 侧对应 `bundle.windows.signCommand`（云签名服务不提供私钥，不能用 `certificateThumbprint`）。

## 4. 明确不做

| 不做 | 原因 |
|---|---|
| EV 证书 | 不再绕过 SmartScreen；贵；需硬件令牌。仅驱动签名 / WHQL 需要 |
| 跨仓签名架构（独立的 releases 仓） | OpenClaw 那样做是因为有基金会、独立发布仓与多人审批；单人项目用一个 GitHub Environment 足够 |
| Flatpak / Snap 签名基础设施 | 只在上架对应商店时才有意义 |
| 自建更新清单签名器 | Tauri updater 的 minisign 已覆盖，重复造只会多一处密钥 |
| 引入 Sparkle | macOS 原生应用的更新方案；当前 Tauri updater 三平台统一，换掉是净损失 |
| `.deb` / `.rpm` 的 GPG 签名 | 只在自建 apt/dnf 源时才有意义；否则只是多一份密钥分发负担 |

## 5. 开放问题

1. ~~**README 与 UI 的 keychain 表述与实现不符。**~~ **已修复（2026-09-13）。**

   代码中**没有任何 keychain 集成**：`Cargo.toml` / `Cargo.lock` / 各 crate 均无
   `keyring` 依赖（`security-framework` 只是 rustls 的传递依赖，用于 TLS 证书校验）。
   密钥实际**明文**存于 `~/.kiwano/kiwano.db`（`providers.api_key`、`api_keys.api_key`、
   以及 `takeover_backups` 里的原始配置），仅靠 Unix 文件权限保护：
   目录 0700 / DB 0600 / `-wal` 0600 / `-shm` 0600，且有测试覆盖
   （`open_restricts_the_database_to_the_owner`，见 `crates/gateway/src/store/mod.rs`）。
   代码注释自己写着 "a keychain is planned, not done"。

   这是对用户的安全承诺，所以按事实改写，而不是留着等 keychain 落地。改动位置：

   | 位置 | 原表述 | 现表述 |
   |---|---|---|
   | `README.md:15` | `Your keys stay in the OS keychain.` | `…on your machine, in an owner-only local database.` |
   | `README.md:85` | `stored in the **OS keychain**` | `stored **locally, in an owner-only SQLite database** — directory 0700, file 0600` |
   | `src/i18n/en/providers.ts:116,118,120` | `API keys stay in the system keychain` | `API keys are stored locally, readable only by you` |
   | `src/i18n/en/addProvider.ts:32` | `Stored in the local keychain only` | `Stored locally, readable only by you` |
   | `src/i18n/zh-CN/providers.ts:99,101,103` | `API 密钥仅保存在系统钥匙串` | `API 密钥仅存本机，只有你能读取` |
   | `src/i18n/zh-CN/addProvider.ts:27` | `仅保存在本机钥匙串` | `仅存本机，只有你能读取` |
   | `site/index.html:583,699,759,779` | 中英两版「钥匙串 / keychain」 | 同为「仅存本机 / a local database」 |

   > ⚠️ **本文原先的表述有一处错误，值得记下来**：原文说「`zh-CN` 资源里没有任何这类
   > 表述（0 处），只需改英文的 4 处加 README 的 2 处」。实际上 `zh-CN` 有 4 处
   > （`providers.ts` 3 处、`addProvider.ts` 1 处），站点另有 4 处。范围比本文说的大一倍，
   > 而且 **UI 里那句话比 README 更直接** —— 用户是在「添加供应商」弹窗里读到它的。
   > 教训：凭「应该没有」下结论前先 grep。
2. **`TAURI_SIGNING_PRIVATE_KEY` 的归一化已经很严，但 Apple 密钥没有同等强度。**
   updater 密钥会校验 base64 分组对齐、`RW` 前缀、行数（见 `release.yml`）；
   `APPLE_CERTIFICATE` 只做了空白剥离。以现状看风险低（Apple blob 的解码错误会直接失败），
   但两者强度不对称。
3. **rehearsal tag 现在会跑 `release-manifest`。** 这是有意的（rehearsal 的价值就在于走通全链路），
   两个产出只落到永不发布的 draft 上。若将来该 job 增加任何会外发的东西，需重新评估。
4. **`EXPECTED_UPDATER_PUBKEY` 的轮换流程未定义。** 若确实需要更换 updater 密钥
   （例如私钥泄露），旧版本 App 只认旧公钥，无法通过更新自举 —— 需要单独发布一次
   带新公钥的版本，且用户必须手动安装。这条路径尚未演练过。
