// Infer a provider's brand icon from its endpoint URL.
// The Apps list stores only logo_char/logo_color (letter avatars); instead of
// persisting an icon per row, the brand mark is derived at render time from
// the endpoint host — the same signal the import flow has. Returns a key from
// the registry in ./index, or null when nothing matches (caller keeps the
// letter-avatar fallback).
import { hasIcon } from "./index";

/** host-substring → registry key; first match wins (specific rules first) */
const RULES: [string, string][] = [
  // Chinese labs / aggregators (specific domain rules before generic vendors)
  ["moonshot", "kimi"],
  ["bigmodel", "zhipu"],
  ["zhipu", "zhipu"],
  ["chatglm", "chatglm"],
  ["dashscope", "qwen"],
  ["bailian", "bailian"],
  ["aliyun", "alibaba"],
  ["minimax", "minimax"],
  ["volces", "doubao"],
  ["volcengine", "doubao"],
  ["doubao", "doubao"],
  ["hunyuan", "hunyuan"],
  ["tencent", "tencent"],
  ["wenxin", "wenxin"],
  ["baidubce", "baidu"],
  ["baidu", "baidu"],
  ["modelscope", "modelscope"],
  ["siliconflow", "siliconflow"],
  ["aihubmix", "aihubmix"],
  ["openrouter", "openrouter"],
  ["packycode", "packycode"],
  ["newapi", "newapi"],
  ["novita", "novita"],
  ["ppio", "ppio"],
  ["stepfun", "stepfun"],
  ["longcat", "longcat"],
  ["xiaomi", "xiaomimimo"],
  ["lingyiwanwu", "yi"],
  ["deepseek", "deepseek"],
  ["kimi", "kimi"],
  ["qwen", "qwen"],
  // International vendors
  ["generativelanguage", "gemini"],
  ["googleapis", "google"],
  ["gemini", "gemini"],
  ["aistudio", "google"],
  ["grok", "grok"],
  ["x.ai", "xai"],
  ["anthropic", "anthropic"],
  ["claude", "claude"],
  ["openai", "openai"],
  ["mistral", "mistral"],
  ["cohere", "cohere"],
  ["perplexity", "perplexity"],
  ["ollama", "ollama"],
  ["nvidia", "nvidia"],
  ["huggingface", "huggingface"],
  ["amazonaws", "aws"],
  ["azure", "azure"],
  ["cloudflare", "cloudflare"],
  ["vercel", "vercel"],
  ["github", "github"],
];

/**
 * Hosts whose *port* names the brand, because the host is the machine itself.
 * A provider on `localhost:11434` is Ollama by default port — the local
 * servers are the one case a host-name rule cannot reach, since every one of
 * them is "localhost". Only ports that a vendor ships as its own default
 * belong here: 8000 and 8080 are everybody's, and guessing there would put a
 * brand on a row that has nothing to do with it.
 */
const LOOPBACK_PORTS: [string, string][] = [["11434", "ollama"]];

/** `localhost`, `127.0.0.1`, `::1`, or a name that resolves within the machine
    as far as a URL can say. */
function isLoopback(host: string): boolean {
  return host === "localhost" || host === "127.0.0.1" || host === "::1" || host === "[::1]";
}

export function iconForEndpoint(endpoint: string): string | null {
  const authority = endpoint
    .trim()
    .toLowerCase()
    .replace(/^https?:\/\//, "")
    .split("/")[0];
  const host = authority.split(":")[0];
  if (!host) return null;
  if (isLoopback(host)) {
    // `host:port`, with IPv6 in brackets — the port is the only signal here.
    const port = authority.match(/:(\d+)$/)?.[1];
    for (const [needle, icon] of LOOPBACK_PORTS) {
      if (port === needle && hasIcon(icon)) return icon;
    }
  }
  for (const [needle, icon] of RULES) {
    if (host.includes(needle) && hasIcon(icon)) return icon;
  }
  return null;
}
