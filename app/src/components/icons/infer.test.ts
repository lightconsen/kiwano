// The endpoint → brand inference: which rows get a real mark instead of the
// letter avatar. Its inputs are user-typed endpoints, so the cases that matter
// are the sloppy spellings and the ones that must stay unclaimed.
import { describe, expect, it } from "vitest";
import { iconForEndpoint } from "./infer";

describe("iconForEndpoint", () => {
  it("reads the vendor out of a host", () => {
    expect(iconForEndpoint("https://api.deepseek.com")).toBe("deepseek");
    expect(iconForEndpoint("https://api.moonshot.cn/v1")).toBe("kimi");
    expect(iconForEndpoint("api.openai.com")).toBe("openai");
  });

  it("reads a local server's brand out of its default port", () => {
    // Every local server is "localhost", so the port is the only signal there
    // is — and 11434 is Ollama's own.
    expect(iconForEndpoint("http://localhost:11434")).toBe("ollama");
    expect(iconForEndpoint("http://127.0.0.1:11434/v1")).toBe("ollama");
    // A port anybody can use claims nothing: a vLLM on 8000 is not a brand.
    expect(iconForEndpoint("http://localhost:8000")).toBeNull();
    // …and the port only means Ollama on the machine itself.
    expect(iconForEndpoint("https://ollama.example.com:11434")).toBe("ollama"); // by name, as before
    expect(iconForEndpoint("https://api.example.com:11434")).toBeNull();
  });

  it("leaves what it cannot name to the letter avatar", () => {
    expect(iconForEndpoint("https://my-relay.internal")).toBeNull();
    expect(iconForEndpoint("")).toBeNull();
    expect(iconForEndpoint("   ")).toBeNull();
  });
});
