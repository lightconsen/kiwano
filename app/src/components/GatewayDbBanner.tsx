// The app and its gateway are on different databases.
//
// Nothing else reports this. A gateway launched by a service manager resolves
// `$HOME/.kiwano/kiwano.db` against *its* `$HOME`, so it can be up, healthy and
// routing while writing to a file nobody reads: every screen then shows a store
// that is not being filled, with no error anywhere (`docs/lessons.md`'s family —
// a silent wrong answer). The identity in `/status` is what makes it visible,
// and this is where a reader is told.
//
// Deliberately not dismissible, unlike the credential bar next to it: dismissing
// a warning whose cause is still there only hides it again. The text says what
// to do, so the bar has somewhere to point.
import { useEffect, useState } from "react";

import { api } from "../api/client";
import { useT } from "../i18n";

export function GatewayDbBanner() {
  const t = useT();
  const [mismatch, setMismatch] = useState(false);

  useEffect(() => {
    let alive = true;
    const pull = () => {
      // Same silent treatment as the credential bar: a status read that fails
      // is not something to report here — `running: false` already says the
      // gateway is not answering, and this bar is about a gateway that *is*.
      try {
        api
          .getGatewayStatus()
          .then((s) => {
            if (alive) setMismatch(s.db_mismatch);
          })
          .catch(() => {});
      } catch {}
    };
    pull();
    const timer = window.setInterval(pull, 30_000);
    return () => {
      alive = false;
      window.clearInterval(timer);
    };
  }, []);

  if (!mismatch) return null;

  return (
    <div
      className="flex items-center gap-2.5 border-b border-line px-4 py-1.5 text-[12px]"
      style={{ background: "color-mix(in srgb, var(--amber) 10%, transparent)" }}
      role="status"
    >
      <span>{t("app.dbMismatch")}</span>
    </div>
  );
}
