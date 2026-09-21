// The subject strip the two agent-settings dialogs share, and the subjects each
// one holds.
import { useT, type KeyPath, type Messages } from "../../i18n";

/** The subjects of one agent's settings, switched between rather than stacked.
    The same segment group the Dashboard's window and the shelf's view use — the
    subjects fit on a row, and naming the other one without opening it is the
    point.

    Generic over which subjects, because the two dialogs hold different ones: a
    built-in agent has files to name, a user-defined one has credentials to hand
    out, and both have ceilings. */
export function SettingsTabs<T extends string>({
  tabs,
  tab,
  onPick,
}: {
  tabs: { id: T; labelKey: KeyPath<Messages> }[];
  tab: T;
  onPick: (t: T) => void;
}) {
  const t = useT();
  return (
    <div className="mt-1 flex overflow-hidden rounded-lg border border-line text-[12px]">
      {tabs.map((x, i) => (
        <button
          key={x.id}
          className={`seg h-7 flex-1 border-line px-3 text-mut${i > 0 ? " border-l" : ""}${tab === x.id ? " active" : ""}`}
          onClick={() => onPick(x.id)}
        >
          {t(x.labelKey)}
        </button>
      ))}
    </div>
  );
}

/** What each dialog's strip holds. The ceiling is in both; the other subject is
    what that kind of agent is from the outside. */
export const AGENT_TABS: { id: "general" | "limit"; labelKey: KeyPath<Messages> }[] = [
  { id: "general", labelKey: "providers.agentGeneralTab" },
  { id: "limit", labelKey: "strategy.limitLabel" },
];
export const ACCESS_TABS: { id: "access" | "limit"; labelKey: KeyPath<Messages> }[] = [
  { id: "access", labelKey: "providers.accessTab" },
  { id: "limit", labelKey: "strategy.limitLabel" },
];
