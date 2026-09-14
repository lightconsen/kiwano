// Which agents the UI knows about, and what to call them.
//
// `AGENTS` is the fixed registry — the CLIs whose config files this app can
// rewrite or detect. A user-defined agent is a name its user chose, and it is
// app state (`SettingsVm.custom_agents`) that says which exist. Keeping the
// resolver here, next to that list, is what lets every screen ask one question
// ("what do I call this agent id?") instead of each holding a copy of the
// registry and a non-null assertion to go with it.
//
// The fallback matters as much as the lookup: an id nothing knows still renders
// as its own letter avatar. That case stopped being an error the moment agents
// could be deleted — their usage rows, their filter entries and their log lines
// stay behind and still have to draw.
import { AGENTS, type AgentId, type AgentMeta } from "../api/types";

/** Palette for derived avatars. Fixed hexes rather than CSS vars: these sit on
    `AgentChip`/`Logo` the same way the registry's own chip colours do. */
const AVATAR_COLORS = ["#4D6BFE", "#0F9D58", "#8B5CF6", "#DB2777", "#EA580C", "#0891B2"];

let custom: AgentMeta[] = [];
const derived = new Map<string, AgentMeta>();

/** Remember the user's own agents (called wherever settings are read). */
export function rememberCustomAgents(list: { id: string; label: string }[]): void {
  custom = list.map((a) => {
    const meta = { ...derive(a.id), label: a.label };
    derived.set(a.id, meta);
    return meta;
  });
}

/** The entry for an agent id: the registry's, the user's own, or a derived one
    for an id neither knows (a deleted agent's rows, or a stale deep link). */
export function agentMeta(id: string): AgentMeta {
  return (
    AGENTS.find((a) => a.id === id) ?? derived.get(id) ?? derive(id)
  );
}

/** Whether `id` is a built-in — an agent whose config this app manages.
    A type guard, so a caller that needs a built-in (the takeover switch, a
    config reader) can prove it instead of casting. */
export function isBuiltinAgent(id: string): id is AgentId {
  return AGENTS.some((a) => a.id === id);
}

/** Every agent, built-ins first: what a multi-select of "which agents does this
    provider serve" has to offer. */
export function allAgentMetas(): AgentMeta[] {
  return [...AGENTS, ...custom];
}

/** A stable placeholder identity for an id nothing knows: the first letter it
    has, tinted by a hash of the whole id so two of them do not look alike. */
function derive(id: string): AgentMeta {
  const letters = id.replace(/[^a-z0-9]/gi, "");
  const hash = [...id].reduce((n, c) => (n * 31 + c.charCodeAt(0)) >>> 0, 7);
  return {
    id,
    label: id,
    chip_char: (letters[0] ?? "?").toUpperCase(),
    chip_color: AVATAR_COLORS[hash % AVATAR_COLORS.length],
  };
}
