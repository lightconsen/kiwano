// Dismissible "a credential left the machine" bar for the app shell.
//
// The finding itself belongs to the gateway (the credential detector writes it
// into the request log); this only polls for the newest one the user has not
// acknowledged. Both dismissing and clicking through acknowledge: the bar is a
// pointer to the log row, and once the reader has seen or skipped it, a *newer*
// finding is the only thing that should raise it again.
import { useEffect, useState } from "react";
import { ShieldAlert, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { api } from "../api/client";
import { useT } from "../i18n";
import type { RequestLogEntry } from "../api/types";

export function CredentialBanner() {
  const t = useT();
  const [finding, setFinding] = useState<RequestLogEntry | null>(null);

  useEffect(() => {
    let alive = true;
    const pull = () => {
      // try/catch, not just .catch: a partial API mock (the shell's tests) has
      // no such function, and the throw is synchronous — same silent treatment
      // as the alert patrol's failures (App.tsx).
      try {
        api.checkCredentialFinding().then((f) => {
          if (alive) setFinding(f);
        }).catch(() => {});
      } catch {}
    };
    pull();
    const timer = setInterval(pull, 60_000);
    return () => {
      alive = false;
      clearInterval(timer);
    };
  }, []);

  if (!finding) return null;

  const ack = () => {
    setFinding(null);
    api.ackCredentialFinding(finding.id).catch(() => {});
  };

  return (
    <div
      className="flex items-center gap-2.5 border-b border-line px-4 py-1.5 text-[12px]"
      style={{ background: "color-mix(in srgb, var(--red) 8%, transparent)" }}
    >
      <ShieldAlert className="h-3.5 w-3.5 shrink-0" style={{ color: "var(--red)" }} />
      <span>
        {t("app.credentialBannerTitle")}{" "}
        {/* The detector's own line names the rules that fired, never the
            matched value — safe to show verbatim. */}
        <span className="font-mono text-mut">{finding.request_notes?.split("\n")[0]}</span>
      </span>
      {/* Acks and navigates in the same click: the hash deep-link lands on the
          Dashboard's Logs card with this row's dialog open. */}
      <Button
        size="xs"
        variant="outline"
        onClick={() => {
          ack();
          window.location.hash = `#dashboard/log/${finding.id}`;
        }}
      >
        {t("app.credentialBannerCta")}
      </Button>
      <button
        aria-label={t("app.credentialBannerDismiss")}
        title={t("app.credentialBannerDismiss")}
        className="ml-auto text-mut hover:text-ink"
        onClick={ack}
      >
        <X className="h-3.5 w-3.5" />
      </button>
    </div>
  );
}
