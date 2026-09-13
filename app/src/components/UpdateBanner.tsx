// Dismissible "a newer version is available" bar for the app shell.
//
// The check itself belongs to the backend: this only reads what the last check
// found, so it shows the same version as the notification and the About block.
// Dismissal is remembered per version in settings, so one release does not
// re-announce itself on every launch — a newer one will.
import { useEffect, useState } from "react";
import { ArrowUpCircle, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { api } from "../api/client";
import { useT } from "../i18n";
import { onUpdateAvailable } from "../lib/updateEvents";
import { startUpdateInstall, useUpdateInstall } from "../lib/updateInstall";
import type { UpdateInfo } from "../api/types";

export function UpdateBanner() {
  const t = useT();
  const [info, setInfo] = useState<UpdateInfo | null>(null);
  const [dismissed, setDismissed] = useState<string | null>(null);
  const install = useUpdateInstall();

  useEffect(() => {
    let alive = true;
    const pull = () => {
      api
        .getPendingUpdate()
        .then((u) => {
          if (alive) setInfo(u);
        })
        .catch(() => {});
    };
    pull();
    api
      .getSettings()
      .then((s) => {
        if (alive) setDismissed(s.dismissed_update ?? null);
      })
      .catch(() => {});
    // A check that runs while the app is open (startup or manual) re-reads it.
    const off = onUpdateAvailable(pull);
    return () => {
      alive = false;
      off();
    };
  }, []);

  if (!info || info.version === dismissed) return null;

  const dismiss = () => {
    setDismissed(info.version);
    api.updateSettings({ dismissed_update: info.version }).catch(() => {});
  };

  return (
    <div
      className="flex items-center gap-2.5 border-b border-line px-4 py-1.5 text-[12px]"
      style={{ background: "var(--kiwi-soft)" }}
    >
      <ArrowUpCircle className="h-3.5 w-3.5 shrink-0" style={{ color: "var(--kiwi)" }} />
      <span>{t("settings.updateAvailable", { version: info.version })}</span>
      {/* Starts the transfer and lands on Settings in the same click: the
          download lives in lib/updateInstall, so the route change does not
          interrupt it and Settings picks the progress up mid-flight. */}
      <Button
        size="xs"
        variant="outline"
        disabled={install.installing}
        onClick={() => {
          startUpdateInstall();
          window.location.hash = "#settings";
        }}
      >
        {install.installing
          ? t("settings.downloading")
          : install.err
            ? t("settings.retryDownload")
            : t("settings.downloadInstall")}
      </Button>
      <button
        aria-label={t("settings.dismissUpdate")}
        title={t("settings.hideUntil", { version: info.version })}
        className="ml-auto text-mut hover:text-ink"
        onClick={dismiss}
      >
        <X className="h-3.5 w-3.5" />
      </button>
    </div>
  );
}
