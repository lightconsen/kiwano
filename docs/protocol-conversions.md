# 协议转换矩阵（docs/protocol-conversions.md）

面向维护者的设计记录：哪些「客户端协议 × 上游协议」组合能走通、哪些被明确拒绝、
哪些组合决定**不做**，以及理由。这里不描述界面，也不进 `scripts/build-docs.mjs`
的 `PAGES`——它不会渲染成网站页面。

## 结论先行

- **已实现四条**：Anthropic `/v1/messages` → OpenAI Chat；OpenAI Responses → Chat；
  OpenAI Chat → Anthropic；OpenAI Responses → Anthropic（经 Chat 组合）。
- **前两条**服务「OpenAI 上游」；**后两条**服务「上游只暴露 `/v1/messages`」——Claude
  官方、只转发 Anthropic 协议的转售商、订阅通道。
- **明确不做**：任何 → Gemini 原生上游（Google 自带 OpenAI 兼容端点）；Anthropic →
  Responses；Chat → Responses；Gemini 入站的任何转换。
- **原则**：转不了就带原因拒绝。半转换的请求是一个看不出错的错误答案。

## 四条 wire，不是三种协议

`crates/gateway/src/protocol.rs` 的 `classify_path` 决定的「协议」只到家族为止；真正
决定请求形态的是 wire：

| wire | 入站路径 | 上游形态 |
| --- | --- | --- |
| Anthropic | `/v1/messages`（含 `count_tokens`）、`/v1/complete` | Messages API |
| OpenAI Chat | `/v1/chat/completions` | Chat Completions |
| OpenAI Responses | `/v1/responses` | Responses API（Codex 0.149 起只会说这个） |
| Gemini 原生 | `/v1beta/models/{model}:{method}` | generateContent / streamGenerateContent |

OpenAI 家族的两种 wire 由 `openai_wire`（migration v30）区分；`Both` 是未声明时的
默认，等价于「上游说什么就是什么」。

## 矩阵

「同 wire」直通不在表里：四条 wire 各自对自己永远可用，包括 `Responses → Chat` 之外的
直通（`/v1/completions`、`/v1/embeddings` 也只直通，不参与转换）。

| 入站 | 上游 | 现状 | 决定 | 理由 |
| --- | --- | --- | --- | --- |
| Anthropic | Chat | ✅ `forward::convert` | — | cc-switch 的历史主线：Claude Code → 任何 OpenAI 兼容供应商 |
| Responses | Chat | ✅ `forward::convert_responses` | — | Codex → Chat-only 上游；不做就是直接拒绝 Codex |
| Chat | Anthropic | ✅ `forward::convert_chat` | — | OpenAI 客户端（Cline、Aider、Continue、OpenCode、Droid…）指向 Anthropic 官方或只开 `/messages` 的转售商 |
| Responses | Anthropic | ✅ `forward::convert_responses_anthropic` | — | Codex 用 Claude 模型：`responses_to_chat` → `chat_to_anthropic`，响应反向经 `anthropic_to_responses` |
| Anthropic | Responses | ❌ | 不做 | Responses-only 的上游几乎只面向 Codex；Anthropic 客户端取 GPT 模型走聚合商，那些上游普遍同时有 Chat |
| Chat | Responses | ❌ | 不做 | 要为 Chat 客户端凭空发明 `input` 与 stored-response 语义，没有客户端要求过 |
| Anthropic / Chat / Responses | Gemini 原生 | ❌ | **不做** | Google 官方提供 OpenAI 兼容端点，声明成 OpenAI provider 即可（见下） |
| Gemini | Anthropic | ❌ | 暂不做 | 只为「Gemini CLI 用第三方模型」存在，用户通常换客户端；四套 wire 里差异最大 |
| Gemini | Chat | ❌ | 暂不做 | 同上 |
| Gemini | Responses | ❌ | 不做 | 没有真实场景 |

## 为什么 → Gemini 上游不做

Gemini API 自带 OpenAI 兼容层：

```
https://generativelanguage.googleapis.com/v1beta/openai/
```

（2026-10-24 核对官方文档：`chat/completions`、`models`、图片与音频输入、embedding
都走这个前缀。）把这样的地址按 OpenAI provider 添加，现有的 Chat 直通就能用，不需要
任何转换代码。

代价是 Gemini 原生特有的东西在兼容层里会被削弱——thought signature、Grounding、
部分原生工具——但那正是「用原生协议的客户端」才需要的东西：Gemini CLI 接管时写的是
`/v1beta` 原生路径，继续直通即可。为同一个厂商再写一套原生适配器是纯重复劳动。

## 为什么恰恰是「→ Anthropic」的两条

唯一无法绕开的场景：**上游只暴露 `/v1/messages`**。

- `api.anthropic.com` 本身没有 OpenAI 形态的端点；
- 只转发 Anthropic 协议的转售商、订阅通道同样如此；
- 而锁定 OpenAI 协议的客户端不能自己改协议。

反过来，Anthropic 客户端用 GPT 模型几乎总经过聚合商（OpenRouter、new-api 等），那些
上游对客户端本来就说 OpenAI 协议——现有直通已经覆盖。

### Responses → Anthropic 是组合出来的

Codex 0.149 起删掉了 chat wire，Kiwano 的接管也强制 `wire_api = "responses"`
（`crates/adapters/src/codex_config/repairs.rs`），所以它需要的是 Responses → Anthropic。
这条没有单独写一套映射，而是复用 Chat 这一对：

- 请求：`responses_to_chat` → `chat_to_anthropic`
- 响应：`anthropic_to_chat` → `chat_to_responses`
- 流式：`create_responses_sse_stream_from_anthropic`
  （Anthropic SSE → Chat chunk → Responses 事件）

代价是继承 `responses_to_chat` 已有的取舍（`previous_response_id`、`store: true`、
`reasoning` item 被拒/丢弃），与今天「Codex 经 Chat-only 上游」的保真度一致。

## 配套缺口（做转换时必须一起决定）

- **`/v1/messages/count_tokens`**：Anthropic 客户端会调它。转换上游目前一律
  `protocol_mismatch`——转换腿只认 `path == "/v1/messages"`。要么估算并按 Anthropic
  形状返回，要么在错误里说清、并确认客户端能容忍失败。
- **Chat / Responses 转 Anthropic 的拒绝项**：`response_format` 的两种 JSON 形式、
  `n > 1`、`logprobs`、音频/文件/未知内容 part、非 function 工具、非法工具名都会得到
  自带原因的 422。调参类字段（penalties、seed、logit_bias、reasoning_effort）被丢弃，
  因为它们在 Anthropic wire 上没有对应物、也不承载内容。
- **`max_tokens`**：Anthropic 必填，客户端没给时补 8192（所有 Claude 型号都接受）。
- **cache 写入**：Responses usage 没有写缓存字段，`build_responses_usage` 现在把
  Anthropic 的 `cache_creation_input_tokens`（连同 1h 切片）透传下去，供计费扫描器读取。
- **`/v1/models`**：两种协议都有，`classify_path` 判为 `Ambiguous`，由 agent 归属解析。
  跨协议的模型列表**不翻译**：转换上游的模型名不在上游列表里时，列表就是上游自己的。
- **`/v1/embeddings`、`/v1/completions`**：只直通，不参与任何转换。
- **附加端点的 wire**：`openai_wire` 只在 provider 主协议是 OpenAI 时允许声明
  （`resolve_openai_wire`）。一个 Anthropic 主协议的 provider 挂的 OpenAI 附加端点
  无法声明成 Chat-only，路由会按 `Both` 对待它。

## 一个 provider 内部的分协议路由

「一个 provider、两组模型、两个端点」不再需要转换就能正确路由：
`crates/gateway/src/forward/inbound.rs` 的 `resolve_inbound` 先看模型，再看协议。

- Hub 条目的 `models.json` 用 `serves: {"anthropic": id}` / `{"openai": id}` 声明每个
  模型属于哪个端点（Command Code 是第一个例子：13 / 80）；
- 路由表把它读成 `UpstreamProvider::endpoint_models`（`router/mod.rs`），
  `endpoint_serves` 回答「这个协议端点服务这个模型吗」；
- 模型在入站协议端点里没有、在另一个端点里有，就去另一个端点：同协议走
  `Alternate`，Anthropic → OpenAI 走 `ConvertAnthropicToOpenAI`；
- 其余情况（没有模型、没有集合、两边都没有）回落到原来的纯协议规则。

这条路径是协议中立的：把 Gemini 加进来也能路由，缺的只是转换函数本身。

## 新增一条转换的步骤

1. `InboundResolution` 增加一个变体，`resolve_inbound` 里只加「严格窄于现有规则」的
   分支；
2. `crates/gateway/src/forward/` 新增 forward 模块，照 `convert.rs` 的结构：请求转换、
   响应转换、SSE、usage 映射、错误透传；
3. 转换函数放进 `crates/adapters/src/proxy/providers/transform/`，按 domain 拆文件，
   并保持 `transform::<name>` 的旧路径（facade re-export）；
4. metering 读**客户端可见的字节**，不是上游形态；
5. compat shim（`forward/shim.rs`）按上游 host 加规则，不要全局生效；
6. 测试：resolve 的每条分支、转换的往返、SSE 事件序列、usage 记账；端到端用本地
   mock 两个端点。

规模参考：现有两对转换（Anthropic→Chat、Responses→Chat）合计约 4,250 行，含流式。
一条新腿大致同量级，Gemini 最高。

## 变更记录

- **2026-10-22（v0.3.6）**：模型级端点选择落地，Command Code 的 13 / 80 分流可用。
- **2026-10-24**：本文记录 N×N 的取舍。补 Chat→Anthropic（优先）与
  Responses→Anthropic（按需）；其余组合不做，理由如上。
- **2026-10-24（实现）**：两条新腿落地——`forward/convert_chat.rs`、
  `forward/convert_responses_anthropic.rs`；转换器在
  `transform/chat_request.rs`、`transform/chat_response.rs`、`providers/chat_streaming.rs`。
  非流式从原始 Anthropic 字节计费（`cache_inclusive: false`），流式从转换后的客户端
  字节计费（`cache_inclusive: true`）。
