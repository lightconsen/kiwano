// Which agents the mock reports as not installed, and the directories a dev
// user has declared for them — the in-memory stand-in for
// `Store::manual_agent_dirs`.

import type { AgentId } from "../types";

/// Which agents the dev fixture reports as *not* installed. Deliberately not
/// all of them: the "Kiwano cannot find this one" menu only has anything to
/// show when something is missing, and these match the versions fixture below.
export const NOT_INSTALLED_AGENTS: AgentId[] = ["gemini", "codebuddy", "kimi", "qwen"];

/// Directories the dev user has declared, keyed by agent id — the in-memory
/// stand-in for `Store::manual_agent_dirs`.
export const declaredDirs: Partial<Record<AgentId, string>> = {};

/// The name an agent's CLI installs as; a couple of registry ids differ.
export const binaryFor = (id: AgentId): string =>
  id === "grokbuild" ? "grok" : id === "claude-desktop" ? "claude" : id;

/// What a declaration reports back. The real backend runs the executable and
/// returns what it printed; the fixture has no filesystem to run, so every
/// declared directory answers with the same plausible version.
export const DEV_DECLARED_VERSION = "1.2.3";
