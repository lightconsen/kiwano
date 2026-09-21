// Agent detection, the declared install directory behind it, and the version
// probe every agent tab reads.

import type { AgentDirHit, AgentId, KiwanoApi } from "../../types";
import { AGENTS } from "../../types";
import { DEV_DECLARED_VERSION, NOT_INSTALLED_AGENTS, binaryFor, declaredDirs } from "../agent_dirs";
import { delay } from "../delay";

export const agentsApi: Pick<
  KiwanoApi,
  | "detectAgents"
  | "agentSearchDirs"
  | "verifyAgentDir"
  | "setAgentDir"
  | "clearAgentDir"
  | "probeAgentVersions"
> = {
  async detectAgents() {
    await delay(120);
    // Dev fixture: most agents installed, a few not — the missing ones are
    // what the "point Kiwano at it" menu exists for, and they match the
    // versions fixture below. The registry's own ids: `a.id` is an `AgentRef`
    // since agents can also be user-defined, and only built-ins are detectable.
    return AGENTS.map((a) => {
      const id = a.id as AgentId;
      const declared = declaredDirs[id];
      if (declared) {
        return {
          agent: id,
          installed: true,
          path: `${declared}/${binaryFor(id)}`,
          manual: true,
        };
      }
      const installed = !NOT_INSTALLED_AGENTS.includes(id);
      return {
        agent: id,
        installed,
        path: installed ? `/usr/local/bin/${binaryFor(id)}` : null,
        manual: false,
      };
    });
  },

  async agentSearchDirs(_agent: AgentId): Promise<string[]> {
    await delay();
    // The real list is the walk's, which is per-platform and per-tool. The
    // fixture answers with the shared prefixes, which is the shape of it.
    return [
      "~/.local/bin",
      "~/.npm-global/bin",
      "~/n/bin",
      "~/.volta/bin",
      "~/.local/share/mise/shims",
      "~/Library/pnpm",
      "/opt/homebrew/bin",
      "/usr/local/bin",
      "/usr/bin",
      "/bin",
      "/usr/sbin",
      "/sbin",
      // A real one, and the reason the list has to truncate: PATH entries are
      // as long as the platform feels like making them.
      "/var/run/com.apple.security.cryptexd/codex.system/bootstrap/usr/local/bin",
    ];
  },

  async verifyAgentDir(agent: AgentId, dir: string): Promise<AgentDirHit> {
    await delay();
    // The real backend runs the executable it finds and reports what it printed.
    // The fixture has no filesystem to look in, so every absolute directory
    // "works" — which leaves the failure branch reachable only by typing a
    // relative path, and the rest of it to the real app.
    const trimmed = dir.trim();
    if (!trimmed.startsWith("/")) throw new Error(`no \`${binaryFor(agent)}\` in ${trimmed}`);
    return { path: `${trimmed}/${binaryFor(agent)}`, version: DEV_DECLARED_VERSION };
  },

  async setAgentDir(agent: AgentId, dir: string): Promise<AgentDirHit> {
    await delay();
    const trimmed = dir.trim();
    if (!trimmed.startsWith("/")) throw new Error(`no \`${binaryFor(agent)}\` in ${trimmed}`);
    declaredDirs[agent] = trimmed;
    return { path: `${trimmed}/${binaryFor(agent)}`, version: DEV_DECLARED_VERSION };
  },

  async clearAgentDir(agent: AgentId): Promise<void> {
    await delay();
    delete declaredDirs[agent];
  },

  async probeAgentVersions() {
    await delay(600);
    // A declared agent is one the probe now finds, so it belongs here too —
    // otherwise a directory the user just pointed at would show a tab with no
    // version, which is not what the real backend does.
    const declared = Object.keys(declaredDirs).map((agent) => ({
      agent: agent as AgentId,
      version: DEV_DECLARED_VERSION,
    }));
    return [
      { agent: "claude", version: "2.1.83 (Claude Code)" },
      { agent: "codex", version: "0.42.0" },
      { agent: "grokbuild", version: "0.9.4" },
      { agent: "claude-desktop", version: null },
      { agent: "opencode", version: "1.0.120" },
      { agent: "openclaw", version: "0.23.1" },
      { agent: "hermes", version: "0.8.2" },
      { agent: "pi", version: "0.5.12" },
      { agent: "cline", version: "3.0.62" },
      ...declared,
    ];
  },
};
