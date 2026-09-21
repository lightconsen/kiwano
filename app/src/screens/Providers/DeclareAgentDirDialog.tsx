// Pointing the detector at a built-in agent it could not find, and reporting
// what it turns up.
import { useEffect, useState } from "react";

import { RefreshCw } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { ProviderLogo } from "@/components/icons/ProviderLogo";
import { open } from "@tauri-apps/plugin-dialog";
import { api } from "../../api/client";
import { useT } from "../../i18n";
import { type AgentDetect, type AgentDirHit, type AgentId } from "../../api/types";

/** Re-probe the machine, resolving with what it found — `null` when the probe
 * itself did not run, which is not the same answer as an empty list. */
export type Redetect = () => void | Promise<AgentDetect[] | null | void>;

/** The directory a resolved binary sits in. A declared directory comes back
 * only as the binary the walk found *in* it, and the dialog has to open on
 * something the user can correct rather than a blank field. */
export function dirOf(binaryPath: string): string {
  const cut = Math.max(binaryPath.lastIndexOf("/"), binaryPath.lastIndexOf("\\"));
  return cut > 0 ? binaryPath.slice(0, cut) : "";
}

/** Point the detector at a built-in agent it could not find.
 *
 * The backend checks the directory before storing anything — it has to hold a
 * runnable executable with the agent's own name — and the note in the dialog
 * says what that check can and cannot establish, rather than implying a
 * verification that has no way to exist.
 */
export function DeclareAgentDirDialog({
  agent,
  onClose,
  onSaved,
  onCleared,
  onRedetect,
}: {
  agent: {
    id: AgentId;
    label: string;
    icon?: string;
    dir?: string;
    declared?: boolean;
    installed?: boolean;
    path?: string | null;
  } | null;
  onClose: () => void;
  onSaved: () => void;
  onCleared: () => void;
  onRedetect?: Redetect;
}) {
  const t = useT();
  const [dir, setDir] = useState("");
  const [busy, setBusy] = useState(false);
  // Two different waits, two flags: "check again" re-probes the machine, and
  // the verdict below is about one directory. Sharing one would put "checking
  // that directory" on screen for a probe that is not looking at it.
  const [checking, setChecking] = useState(false);
  const [checkingDir, setCheckingDir] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  // What the last "check again" came back with: the machine's own answer, as
  // opposed to the verdict about one directory below. Without it a re-probe that
  // finds nothing leaves no trace at all — the click would look ignored.
  const [recheck, setRecheck] = useState<"missing" | "no-answer" | null>(null);
  // What a picked directory turned out to hold. Set when the picker returns,
  // rather than when the user finally presses Add: the picker is where they are
  // asked to name a directory, so it is where a wrong one should be answered.
  const [checked, setChecked] = useState<{ hit?: AgentDirHit; reason?: string } | null>(null);
  // The probe found it after all — the user installed it, fixed a PATH, or
  // pointed Kiwano at it from somewhere else. Then there is nothing to declare:
  // the field and the list go away and the dialog says so, rather than leaving
  // the user to work out that the tab behind it is the answer.
  const found = !agent?.declared && agent?.installed === true;
  // Where the detector looked, for the agents it did not find: "we cannot find
  // it" is only worth believing next to the list it is measured against — and
  // a stale shell PATH is exactly the kind of thing the list makes visible.
  const [searched, setSearched] = useState<string[]>([]);
  useEffect(() => {
    if (!agent) return;
    setDir(agent.dir ?? "");
    setErr(null);
    setChecked(null);
    setRecheck(null);
    setSearched([]);
    if (agent.declared) return;
    let live = true;
    void api.agentSearchDirs(agent.id).then(
      (dirs) => live && setSearched(dirs),
      // A list that could not be read is no list: the dialog says what it can
      // and offers the field, which is the part that matters.
      () => live && setSearched([]),
    );
    return () => {
      live = false;
    };
    // Keyed on the agent's identity, not on the `agent` object: the parent
    // rebuilds that object on every render (it carries the live probe result),
    // and depending on it would run this reset — clearing the field, the picked
    // directory's verdict and the re-probe's — every time anything upstream
    // re-rendered.
  }, [agent?.id, agent?.declared]);
  const save = async () => {
    if (!agent || !dir.trim() || busy) return;
    setBusy(true);
    setErr(null);
    // The stored check is this attempt's business: if the binary went away
    // between the pick and this click, a green line would be vouching for a
    // path the write is about to refuse.
    setChecked(null);
    try {
      await api.setAgentDir(agent.id, dir.trim());
      onSaved();
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };
  /** Drop the declaration. The dialog stays open on a failure, the same way a
   * failed save does — the agent still has the directory it had. */
  const forget = async () => {
    if (!agent || busy) return;
    setBusy(true);
    setErr(null);
    try {
      await api.clearAgentDir(agent.id);
      onCleared();
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };
  /** Check one directory without storing it. The backend runs whatever it finds
   * there — that is what "does this command work" means — so this is a side
   * effect, and it is run for a directory the user picked, never per keystroke. */
  const check = async (candidate: string) => {
    if (!agent || !candidate.trim()) return;
    setCheckingDir(true);
    setChecked(null);
    try {
      setChecked({ hit: await api.verifyAgentDir(agent.id, candidate.trim()) });
    } catch (e) {
      setChecked({ reason: e instanceof Error ? e.message : String(e) });
    } finally {
      setCheckingDir(false);
    }
  };
  const browse = async () => {
    try {
      const picked = await open({ directory: true, multiple: false });
      if (typeof picked === "string") {
        setDir(picked);
        void check(picked);
      }
    } catch {
      // A desktop-only affordance, and absent under `pnpm dev`. Typing the
      // path is the fallback, so a picker that will not open is not an error.
    }
  };
  /** Ask the machine again — the whole point being that the user may have just
   * installed the tool, which is not something Kiwano can notice on its own. */
  const checkAgain = async () => {
    if (checking) return;
    setChecking(true);
    setErr(null);
    setRecheck(null);
    try {
      const found = await onRedetect?.();
      // The answer is read off what the probe returned, never off the
      // `agentDetect` prop: reading the prop here would race the very update
      // this call is waiting for, and a probe that *did* find the agent would
      // flash "still not found" for a frame before the prop landed.
      setRecheck(
        Array.isArray(found)
          ? found.some((d) => d.agent === agent?.id && d.installed)
            ? null // Found: the dialog switches to its found state instead.
            : "missing"
          : "no-answer",
      );
    } finally {
      setChecking(false);
    }
  };
  return (
    <Dialog open={agent != null} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="max-w-[420px]">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2 text-[13px]">
            {/* The mark is decoration here — the title already names the agent,
                and the logo would read its name a second time. */}
            {agent?.icon && (
              <span aria-hidden className="inline-flex shrink-0">
                <ProviderLogo icon={agent.icon} name={agent.label} size={16} />
              </span>
            )}
            {found
              ? t("providers.foundTitle")
              : agent?.declared
                ? t("providers.declaredAgentTitle", { agent: agent.label })
                : t("providers.addAgentTitle", { agent: agent?.label ?? "" })}
          </DialogTitle>
        </DialogHeader>
        {found ? (
          // Found after all: nothing to point at, and nothing left to ask.
          <div className="min-w-0 space-y-3 px-5 pb-4 pt-1">
            <p className="text-[11px] leading-relaxed text-mut">{t("providers.foundBody")}</p>
            {agent?.path && <FoundPath path={agent.path} />}
            <div className="flex justify-end pt-1">
              <Button
                size="sm"
                className="h-7 px-3 text-[12px] font-semibold"
                onClick={onClose}
              >
                {t("common.close")}
              </Button>
            </div>
          </div>
        ) : (
        // The dialog this control exists for: Kiwano cannot find it, so the
        // user points at the directory.
        <div className="min-w-0 space-y-3 px-5 pb-4 pt-1">
          <div>
            <Label className="text-[11px] font-medium text-mut">{t("providers.installDir")}</Label>
            <div className="mt-1 flex items-center gap-1.5">
              <Input
                autoFocus
                aria-label={t("providers.installDir")}
                className="h-8 text-[12px]"
                value={dir}
                placeholder={t("providers.installDirPlaceholder")}
                onChange={(e) => {
                  setDir(e.target.value);
                  // The verdict was about the path that was there a keystroke
                  // ago, and leaving it up would vouch for this one.
                  setChecked(null);
                }}
                onKeyDown={(e) => e.key === "Enter" && void save()}
              />
              <Button
                variant="outline"
                size="sm"
                className="h-8 shrink-0 text-[12px]"
                onClick={() => void browse()}
              >
                {t("providers.browse")}
              </Button>
            </div>
          </div>
          {(checkingDir || checked) && (
            <div className="min-w-0">
              {checkingDir ? (
                <p className="flex items-center gap-1.5 text-[11px] text-mut">
                  <RefreshCw className="h-3 w-3 animate-spin" />
                  {t("providers.checkingDir")}
                </p>
              ) : checked?.hit ? (
                <>
                  <p className="text-[11px]" style={{ color: "var(--kiwi)" }}>
                    {t("providers.dirOk", { version: checked.hit.version })}
                  </p>
                  <div className="mt-1">
                    <FoundPath path={checked.hit.path} />
                  </div>
                </>
              ) : (
                <p className="text-[11px]" style={{ color: "var(--red)" }}>
                  {checked?.reason}
                </p>
              )}
            </div>
          )}
          {searched.length > 0 && !checked?.hit && (
            // The list scrolls at a whole number of rows: a half-row at the
            // edge reads as a clipped list rather than a scrollable one.
            //
            // It goes once a directory checks out: the question has been
            // answered, and "looked in N directories, and not found" sitting
            // under "found a working executable" reads as a contradiction.
            //
            // `min-w-0` all the way down is what makes the truncation below
            // work: the dialog is a grid, and a grid item will not shrink
            // below its content unless it is told it may — so a PATH entry
            // long enough (Apple's cryptex ones are) would push the list, and
            // with it the dialog, wider than the dialog itself.
            <div className="min-w-0 pl-1.5">
              <p className="text-[11px] font-medium text-mut">
                {t("providers.searchedDirs", { count: searched.length })}
              </p>
              <ul className="mt-1 max-h-32 w-full min-w-0 overflow-x-hidden overflow-y-auto rounded-md bg-surface2 px-1.5 py-1">
                {searched.map((d) => (
                  <li
                    key={d}
                    title={d}
                    className="truncate font-mono text-[10.5px] leading-5 text-mut"
                  >
                    {d}
                  </li>
                ))}
              </ul>
              </div>
            )}
            {recheck && (
              <p className="text-[11px] text-mut">
                {recheck === "missing"
                  ? t("providers.recheckMissing")
                  : t("providers.recheckNoAnswer")}
              </p>
            )}
            <p className="text-[11px] leading-relaxed text-mut">
              {agent?.declared ? t("providers.declaredAgentNote") : t("providers.addAgentNote")}
            </p>
            {err && (
              <p className="text-[11px]" style={{ color: "var(--red)" }}>
                {err}
              </p>
            )}
            <div className="flex items-center justify-end gap-2 pt-1">
              {/* One secondary slot, two meanings: drop the declaration, or ask
                  the machine again for an agent Kiwano never found. */}
              {agent?.declared ? (
                <Button
                  variant="ghost"
                  size="sm"
                  className="mr-auto h-7 px-3 text-[12px] text-mut"
                  disabled={busy}
                  onClick={() => void forget()}
                >
                  {t("common.remove")}
                </Button>
              ) : (
                <Button
                  variant="ghost"
                  size="sm"
                  className="mr-auto h-7 px-3 text-[12px] text-mut"
                  aria-label={t("providers.checkAgain")}
                  title={t("providers.checkAgainTitle")}
                  disabled={checking}
                  onClick={() => void checkAgain()}
                >
                  <RefreshCw className={`h-3 w-3${checking ? " animate-spin" : ""}`} />
                  {t("providers.checkAgain")}
                </Button>
              )}
              <Button variant="ghost" size="sm" className="h-7 px-3 text-[12px]" onClick={onClose}>
                {t("common.cancel")}
              </Button>
              <Button
                size="sm"
                className="h-7 px-3 text-[12px] font-semibold"
                disabled={!dir.trim() || busy}
                onClick={() => void save()}
              >
                {t("providers.addAgent")}
              </Button>
          </div>
        </div>
        )}
      </DialogContent>
    </Dialog>
  );
}

/** The binary the probe just resolved, as a path the user can compare against
    where they think they put it. Truncated in the middle of the dialog, full
    value on hover — the same treatment the searched-directories list gets. */
function FoundPath({ path }: { path: string }) {
  return (
    <p
      title={path}
      className="truncate rounded-md bg-surface2 px-1.5 py-1 font-mono text-[10.5px] text-mut"
    >
      {path}
    </p>
  );
}
