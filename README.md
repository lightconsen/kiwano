# Kiwano

本地优先的 AI Provider 管理器 —— 在一台机器上统一管理多个 AI 服务商与 Agent 的接入。

## 功能

- **Provider 管理** —— 添加、切换、删除供应商配置;网关路由支持 failover / roundrobin / timewindow / quota 等策略。
- **Agent 接管** —— 一键将 Claude Code、Codex、Gemini CLI 等 Agent 接入本地网关(`127.0.0.1:8317`);关闭后自动恢复该 Agent 的原配置。
- **本地网关** —— 单端口常驻,协议族归一(anthropic / openai / gemini),请求经过网关完成用量计量与路由。
- **用量仪表盘** —— 请求数、tokens、费用、延迟的 7 天趋势,按 Provider / Agent 归因。
- **供应商货架** —— 内置 Hub 目录(177 个供应商,支持官方 / 聚合 / 三方 / 免费筛选),一键添加。
- **cc-switch 导入** —— 读取 cc-switch 的既有配置,无缝迁移。

## 技术栈

- 前端:React 19 + TypeScript + Tailwind v4 + shadcn/ui(Vite 构建)
- 桌面:Tauri 2(macOS / Windows / Linux)
- 网关与适配器:Rust(tokio),SQLite 存储

## 项目结构

```
src/            前端源码(React)
src-tauri/      Tauri 主进程
crates/gateway  本地网关(路由 / 策略 / 计量)
crates/adapters Agent 配置适配器(协议归一)
crates/cli      命令行工具
design/         设计稿 SSOT(design.md 为唯一事实来源)
```

## 开发

```bash
pnpm install
pnpm dev         # 前端开发服务
pnpm tauri dev   # 桌面应用开发模式
pnpm build       # 前端生产构建
```

## 许可

Apache License 2.0,详见 [LICENSE](LICENSE)。

部分模块移植自 [cc-switch](https://github.com/farion1231/cc-switch)(MIT),移植文件均携带上游声明,详见 [THIRD-PARTY-NOTICES](THIRD-PARTY-NOTICES)。
